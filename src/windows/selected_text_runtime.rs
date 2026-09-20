use std::mem::size_of;
use std::ptr::{copy_nonoverlapping, null_mut};
use std::thread::sleep;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::GlobalFree;
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, CountClipboardFormats, EmptyClipboard, GetClipboardData,
    GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows_sys::Win32::System::Ole::{CF_DIB, CF_DIBV5, CF_HDROP, CF_UNICODETEXT};

use super::keyboard_runtime::{inject_ctrl_chord, inject_selected_text};
use crate::replacement::{SelectedReplacementAction, SelectedText};

const COPY_TIMEOUT: Duration = Duration::from_millis(700);
const CLIPBOARD_OPEN_TIMEOUT: Duration = Duration::from_millis(300);
const RETRY_INTERVAL: Duration = Duration::from_millis(5);
const VK_C: u16 = b'C' as u16;
const PREFERRED_DROP_EFFECT_NAME: &str = "Preferred DropEffect";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectedTextRuntimeError {
    ClipboardSnapshotFailed,
    UnsupportedClipboardState,
    ClipboardRestoreVerificationFailed,
    ClipboardOpenTimeout,
    ClipboardReadFailed,
    ClipboardWriteFailed,
    ClipboardTextDecodeFailed,
    KeyboardInjection(super::RuntimeError),
    ActionDoesNotMatchSelection,
}

impl From<super::RuntimeError> for SelectedTextRuntimeError {
    fn from(value: super::RuntimeError) -> Self {
        Self::KeyboardInjection(value)
    }
}

pub struct SelectedTextSession {
    selected: SelectedText,
}

impl SelectedTextSession {
    pub fn capture() -> Result<Option<Self>, SelectedTextRuntimeError> {
        let mut snapshot = ClipboardSnapshot::capture()?;
        let sequence_before_copy = unsafe { GetClipboardSequenceNumber() };

        inject_ctrl_chord(VK_C)?;
        if !wait_for_clipboard_change(sequence_before_copy, COPY_TIMEOUT) {
            snapshot.restore()?;
            return Ok(None);
        }

        let copied_text = read_unicode_clipboard();
        snapshot.restore()?;

        let Some(text) = copied_text? else {
            return Ok(None);
        };
        let Ok(selected) = SelectedText::try_new(text) else {
            return Ok(None);
        };

        Ok(Some(Self { selected }))
    }

    pub fn selected_text(&self) -> &SelectedText {
        &self.selected
    }

    pub fn apply(self, action: &SelectedReplacementAction) -> Result<(), SelectedTextRuntimeError> {
        if action.source() != &self.selected {
            return Err(SelectedTextRuntimeError::ActionDoesNotMatchSelection);
        }

        inject_selected_text(action.replacement().as_str())?;
        Ok(())
    }

    pub fn finish_without_replacement(self) -> Result<(), SelectedTextRuntimeError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClipboardSnapshotMode {
    Empty,
    UnicodeText,
    FileDrop,
    Image,
    Unsupported,
}

pub(super) fn classify_clipboard_snapshot(
    format_count: i32,
    unicode_text_available: bool,
    file_drop_available: bool,
    image_available: bool,
) -> ClipboardSnapshotMode {
    if format_count == 0 {
        ClipboardSnapshotMode::Empty
    } else if file_drop_available {
        ClipboardSnapshotMode::FileDrop
    } else if image_available {
        ClipboardSnapshotMode::Image
    } else if unicode_text_available {
        ClipboardSnapshotMode::UnicodeText
    } else {
        ClipboardSnapshotMode::Unsupported
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileClipboardSnapshot {
    hdrop: Vec<u8>,
    preferred_drop_effect: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImageClipboardFormatSnapshot {
    format: u32,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImageClipboardSnapshot {
    formats: Vec<ImageClipboardFormatSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClipboardSnapshotKind {
    Empty,
    UnicodeText(String),
    FileDrop(FileClipboardSnapshot),
    Image(ImageClipboardSnapshot),
}

struct ClipboardSnapshot {
    kind: ClipboardSnapshotKind,
    restored: bool,
}

impl ClipboardSnapshot {
    fn capture() -> Result<Self, SelectedTextRuntimeError> {
        let format_count = unsafe { CountClipboardFormats() };
        let unicode_text_available =
            unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT as u32) } != 0;
        let file_drop_available = unsafe { IsClipboardFormatAvailable(CF_HDROP as u32) } != 0;
        let image_available = unsafe {
            IsClipboardFormatAvailable(CF_DIBV5 as u32) != 0
                || IsClipboardFormatAvailable(CF_DIB as u32) != 0
        };

        let kind = match classify_clipboard_snapshot(
            format_count,
            unicode_text_available,
            file_drop_available,
            image_available,
        ) {
            ClipboardSnapshotMode::Empty => ClipboardSnapshotKind::Empty,
            ClipboardSnapshotMode::UnicodeText => {
                let Some(text) = read_unicode_clipboard()? else {
                    return Err(SelectedTextRuntimeError::ClipboardSnapshotFailed);
                };
                ClipboardSnapshotKind::UnicodeText(text)
            }
            ClipboardSnapshotMode::FileDrop => {
                ClipboardSnapshotKind::FileDrop(capture_file_clipboard()?)
            }
            ClipboardSnapshotMode::Image => {
                ClipboardSnapshotKind::Image(capture_image_clipboard()?)
            }
            ClipboardSnapshotMode::Unsupported => {
                return Err(SelectedTextRuntimeError::UnsupportedClipboardState);
            }
        };

        Ok(Self {
            kind,
            restored: false,
        })
    }

    fn restore(&mut self) -> Result<(), SelectedTextRuntimeError> {
        if self.restored {
            return Ok(());
        }

        match &self.kind {
            ClipboardSnapshotKind::Empty => clear_clipboard()?,
            ClipboardSnapshotKind::UnicodeText(text) => set_unicode_clipboard(text)?,
            ClipboardSnapshotKind::FileDrop(file) => set_file_clipboard(file)?,
            ClipboardSnapshotKind::Image(image) => set_image_clipboard(image)?,
        }

        if !clipboard_snapshot_matches(&self.kind)? {
            return Err(SelectedTextRuntimeError::ClipboardRestoreVerificationFailed);
        }
        self.restored = true;
        Ok(())
    }
}

impl Drop for ClipboardSnapshot {
    fn drop(&mut self) {
        if !self.restored {
            let _ = self.restore();
        }
    }
}

fn capture_file_clipboard() -> Result<FileClipboardSnapshot, SelectedTextRuntimeError> {
    let _clipboard = ClipboardOpenGuard::open()?;
    let hdrop = read_open_hglobal_bytes(CF_HDROP as u32)?;
    let preferred_format = preferred_drop_effect_format()?;
    let preferred_drop_effect = if unsafe { IsClipboardFormatAvailable(preferred_format) } != 0 {
        Some(read_open_hglobal_bytes(preferred_format)?)
    } else {
        None
    };

    Ok(FileClipboardSnapshot {
        hdrop,
        preferred_drop_effect,
    })
}

fn capture_image_clipboard() -> Result<ImageClipboardSnapshot, SelectedTextRuntimeError> {
    let _clipboard = ClipboardOpenGuard::open()?;
    let mut formats = Vec::new();

    for format in [CF_DIBV5 as u32, CF_DIB as u32] {
        if unsafe { IsClipboardFormatAvailable(format) } != 0 {
            formats.push(ImageClipboardFormatSnapshot {
                format,
                bytes: read_open_hglobal_bytes(format)?,
            });
        }
    }

    if formats.is_empty() {
        return Err(SelectedTextRuntimeError::ClipboardSnapshotFailed);
    }

    Ok(ImageClipboardSnapshot { formats })
}

fn clipboard_snapshot_matches(
    expected: &ClipboardSnapshotKind,
) -> Result<bool, SelectedTextRuntimeError> {
    match expected {
        ClipboardSnapshotKind::Empty => Ok(unsafe { CountClipboardFormats() } == 0),
        ClipboardSnapshotKind::UnicodeText(expected_text) => {
            let actual = read_unicode_clipboard()?;
            Ok(actual.as_deref() == Some(expected_text.as_str()))
        }
        ClipboardSnapshotKind::FileDrop(expected_file) => {
            let _clipboard = ClipboardOpenGuard::open()?;
            if unsafe { IsClipboardFormatAvailable(CF_HDROP as u32) } == 0 {
                return Ok(false);
            }
            if read_open_hglobal_bytes(CF_HDROP as u32)? != expected_file.hdrop {
                return Ok(false);
            }

            if let Some(expected_effect) = &expected_file.preferred_drop_effect {
                let preferred_format = preferred_drop_effect_format()?;
                if unsafe { IsClipboardFormatAvailable(preferred_format) } == 0 {
                    return Ok(false);
                }
                if read_open_hglobal_bytes(preferred_format)? != *expected_effect {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        ClipboardSnapshotKind::Image(expected_image) => {
            let _clipboard = ClipboardOpenGuard::open()?;
            for expected_format in &expected_image.formats {
                if unsafe { IsClipboardFormatAvailable(expected_format.format) } == 0 {
                    return Ok(false);
                }
                if read_open_hglobal_bytes(expected_format.format)? != expected_format.bytes {
                    return Ok(false);
                }
            }
            Ok(true)
        }
    }
}

fn wait_for_clipboard_change(previous: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if unsafe { GetClipboardSequenceNumber() } != previous {
            return true;
        }
        sleep(RETRY_INTERVAL);
    }
    false
}

fn read_unicode_clipboard() -> Result<Option<String>, SelectedTextRuntimeError> {
    if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT as u32) } == 0 {
        return Ok(None);
    }

    let _clipboard = ClipboardOpenGuard::open()?;
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT as u32) };
    if handle.is_null() {
        return Err(SelectedTextRuntimeError::ClipboardReadFailed);
    }

    let bytes = unsafe { GlobalSize(handle) };
    if bytes < size_of::<u16>() {
        return Err(SelectedTextRuntimeError::ClipboardReadFailed);
    }
    let units_len = bytes / size_of::<u16>();
    let pointer = unsafe { GlobalLock(handle) } as *const u16;
    if pointer.is_null() {
        return Err(SelectedTextRuntimeError::ClipboardReadFailed);
    }

    let units = unsafe { std::slice::from_raw_parts(pointer, units_len) };
    let content_len = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units_len);
    let decoded = String::from_utf16(&units[..content_len])
        .map_err(|_| SelectedTextRuntimeError::ClipboardTextDecodeFailed);
    unsafe {
        GlobalUnlock(handle);
    }
    decoded.map(Some)
}

fn read_open_hglobal_bytes(format: u32) -> Result<Vec<u8>, SelectedTextRuntimeError> {
    let handle = unsafe { GetClipboardData(format) };
    if handle.is_null() {
        return Err(SelectedTextRuntimeError::ClipboardSnapshotFailed);
    }
    let size = unsafe { GlobalSize(handle) };
    if size == 0 {
        return Err(SelectedTextRuntimeError::ClipboardSnapshotFailed);
    }
    let pointer = unsafe { GlobalLock(handle) } as *const u8;
    if pointer.is_null() {
        return Err(SelectedTextRuntimeError::ClipboardSnapshotFailed);
    }
    let bytes = unsafe { std::slice::from_raw_parts(pointer, size) }.to_vec();
    unsafe {
        GlobalUnlock(handle);
    }
    Ok(bytes)
}

fn set_unicode_clipboard(text: &str) -> Result<(), SelectedTextRuntimeError> {
    let units = unicode_clipboard_units(text);
    let bytes = units.len() * size_of::<u16>();
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) };
    if memory.is_null() {
        return Err(SelectedTextRuntimeError::ClipboardWriteFailed);
    }

    let pointer = unsafe { GlobalLock(memory) } as *mut u16;
    if pointer.is_null() {
        unsafe {
            GlobalFree(memory);
        }
        return Err(SelectedTextRuntimeError::ClipboardWriteFailed);
    }
    unsafe {
        copy_nonoverlapping(units.as_ptr(), pointer, units.len());
        GlobalUnlock(memory);
    }

    let clipboard = match ClipboardOpenGuard::open() {
        Ok(clipboard) => clipboard,
        Err(error) => {
            unsafe {
                GlobalFree(memory);
            }
            return Err(error);
        }
    };
    if unsafe { EmptyClipboard() } == 0 {
        drop(clipboard);
        unsafe {
            GlobalFree(memory);
        }
        return Err(SelectedTextRuntimeError::ClipboardWriteFailed);
    }
    if unsafe { SetClipboardData(CF_UNICODETEXT as u32, memory) }.is_null() {
        drop(clipboard);
        unsafe {
            GlobalFree(memory);
        }
        return Err(SelectedTextRuntimeError::ClipboardWriteFailed);
    }

    Ok(())
}

fn set_file_clipboard(file: &FileClipboardSnapshot) -> Result<(), SelectedTextRuntimeError> {
    let clipboard = ClipboardOpenGuard::open()?;
    if unsafe { EmptyClipboard() } == 0 {
        return Err(SelectedTextRuntimeError::ClipboardWriteFailed);
    }

    set_open_hglobal_bytes(CF_HDROP as u32, &file.hdrop)?;
    if let Some(effect) = &file.preferred_drop_effect {
        let preferred_format = preferred_drop_effect_format()?;
        set_open_hglobal_bytes(preferred_format, effect)?;
    }

    drop(clipboard);
    Ok(())
}

fn set_image_clipboard(image: &ImageClipboardSnapshot) -> Result<(), SelectedTextRuntimeError> {
    let clipboard = ClipboardOpenGuard::open()?;
    if unsafe { EmptyClipboard() } == 0 {
        return Err(SelectedTextRuntimeError::ClipboardWriteFailed);
    }

    for format in &image.formats {
        set_open_hglobal_bytes(format.format, &format.bytes)?;
    }

    drop(clipboard);
    Ok(())
}

fn set_open_hglobal_bytes(format: u32, bytes: &[u8]) -> Result<(), SelectedTextRuntimeError> {
    let memory = allocate_global_bytes(bytes)?;
    if unsafe { SetClipboardData(format, memory) }.is_null() {
        unsafe {
            GlobalFree(memory);
        }
        return Err(SelectedTextRuntimeError::ClipboardWriteFailed);
    }
    Ok(())
}

fn allocate_global_bytes(bytes: &[u8]) -> Result<*mut core::ffi::c_void, SelectedTextRuntimeError> {
    if bytes.is_empty() {
        return Err(SelectedTextRuntimeError::ClipboardWriteFailed);
    }

    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) };
    if memory.is_null() {
        return Err(SelectedTextRuntimeError::ClipboardWriteFailed);
    }
    let pointer = unsafe { GlobalLock(memory) } as *mut u8;
    if pointer.is_null() {
        unsafe {
            GlobalFree(memory);
        }
        return Err(SelectedTextRuntimeError::ClipboardWriteFailed);
    }
    unsafe {
        copy_nonoverlapping(bytes.as_ptr(), pointer, bytes.len());
        GlobalUnlock(memory);
    }
    Ok(memory)
}

fn preferred_drop_effect_format() -> Result<u32, SelectedTextRuntimeError> {
    let wide: Vec<u16> = PREFERRED_DROP_EFFECT_NAME
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let format = unsafe { RegisterClipboardFormatW(wide.as_ptr()) };
    if format == 0 {
        return Err(SelectedTextRuntimeError::ClipboardSnapshotFailed);
    }
    Ok(format)
}

fn clear_clipboard() -> Result<(), SelectedTextRuntimeError> {
    let _clipboard = ClipboardOpenGuard::open()?;
    if unsafe { EmptyClipboard() } == 0 {
        return Err(SelectedTextRuntimeError::ClipboardWriteFailed);
    }
    Ok(())
}

struct ClipboardOpenGuard;

impl ClipboardOpenGuard {
    fn open() -> Result<Self, SelectedTextRuntimeError> {
        let deadline = Instant::now() + CLIPBOARD_OPEN_TIMEOUT;
        loop {
            if unsafe { OpenClipboard(null_mut()) } != 0 {
                return Ok(Self);
            }
            if Instant::now() >= deadline {
                return Err(SelectedTextRuntimeError::ClipboardOpenTimeout);
            }
            sleep(RETRY_INTERVAL);
        }
    }
}

impl Drop for ClipboardOpenGuard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}

pub(super) fn unicode_clipboard_units(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
