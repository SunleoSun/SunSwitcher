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

use super::caret_locator::CaretLocator;
use super::keyboard_runtime::{
    inject_backward_selection, inject_ctrl_chord, inject_ctrl_shift_chord,
    inject_current_caret_forward_replacement, inject_current_caret_forward_selection,
    inject_key_press, inject_key_presses, inject_previous_caret_range_replacement,
    inject_selected_text,
};
use crate::replacement::{SelectedReplacementAction, SelectedText};

const COPY_TIMEOUT: Duration = Duration::from_millis(700);
const CLIPBOARD_OPEN_TIMEOUT: Duration = Duration::from_millis(300);
const RETRY_INTERVAL: Duration = Duration::from_millis(5);
const VK_C_KEY: u16 = b'C' as u16;
const VK_V_KEY: u16 = b'V' as u16;
const VK_INSERT_KEY: u16 = 0x2D;
const VK_RIGHT_KEY: u16 = 0x27;
const PREVIOUS_TEXT_LOOKBACK: usize = 64;
const PREFERRED_DROP_EFFECT_NAME: &str = "Preferred DropEffect";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardImagePayload {
    format: String,
    data: Vec<u8>,
}

impl ClipboardImagePayload {
    pub fn new(format: String, data: Vec<u8>) -> Self {
        Self { format, data }
    }

    pub fn format(&self) -> &str {
        &self.format
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservableClipboardContent {
    Text(String),
    Image(ClipboardImagePayload),
}

#[derive(Debug)]
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
    SelectionChangedDuringCapture,
}

impl From<super::RuntimeError> for SelectedTextRuntimeError {
    fn from(value: super::RuntimeError) -> Self {
        Self::KeyboardInjection(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TrailingNonWhitespaceRange {
    pub(super) leading_chars: usize,
    pub(super) selected_chars: usize,
    pub(super) trailing_chars: usize,
}

pub(super) fn trailing_non_whitespace_range(text: &str) -> Option<TrailingNonWhitespaceRange> {
    let chars = text.chars().collect::<Vec<_>>();
    let last_non_whitespace = chars
        .iter()
        .rposition(|character| !character.is_whitespace())?;
    let end = last_non_whitespace + 1;
    let start = chars[..end]
        .iter()
        .rposition(|character| character.is_whitespace())
        .map_or(0, |index| index + 1);
    Some(TrailingNonWhitespaceRange {
        leading_chars: start,
        selected_chars: end - start,
        trailing_chars: chars.len() - end,
    })
}

#[cfg(test)]
pub(super) fn is_single_line_capture(text: &str) -> bool {
    !text.contains(['\r', '\n'])
}

pub(super) fn caret_navigation_steps(text: &str) -> usize {
    let mut chars = text.chars().peekable();
    let mut steps = 0usize;
    while let Some(character) = chars.next() {
        if character == '\r' && chars.peek() == Some(&'\n') {
            chars.next();
        }
        steps = steps.saturating_add(1);
    }
    steps
}

pub(super) fn replace_copied_chunk_range(
    text: &str,
    range: TrailingNonWhitespaceRange,
    replacement: &str,
) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let start = range.leading_chars;
    let end = range.leading_chars + range.selected_chars;
    let prefix = chars[..start].iter().collect::<String>();
    let suffix = chars[end..].iter().collect::<String>();
    format!("{prefix}{replacement}{suffix}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum SelectionCaptureIntent {
    #[default]
    Unknown,
    UserSelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyShortcut {
    CtrlInsert,
    CtrlShiftC,
    CtrlC,
}

impl CopyShortcut {
    fn copy_to_clipboard(self) -> Result<(), SelectedTextRuntimeError> {
        match self {
            Self::CtrlInsert => inject_ctrl_chord(VK_INSERT_KEY)?,
            Self::CtrlShiftC => inject_ctrl_shift_chord(VK_C_KEY)?,
            Self::CtrlC => inject_ctrl_chord(VK_C_KEY)?,
        }
        Ok(())
    }

    const fn label(self) -> &'static str {
        match self {
            Self::CtrlInsert => "Ctrl+Insert",
            Self::CtrlShiftC => "Ctrl+Shift+C",
            Self::CtrlC => "Ctrl+C",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectedTextOrigin {
    ExistingSelection {
        retained_after_copy: bool,
    },
    PreviousCaretChunk {
        range: TrailingNonWhitespaceRange,
        retained_after_copy: bool,
    },
}

pub struct SelectedTextSession {
    selected: SelectedText,
    origin: SelectedTextOrigin,
    previous_caret_chunk: Option<String>,
}

impl SelectedTextSession {
    pub fn capture() -> Result<Option<Self>, SelectedTextRuntimeError> {
        Self::capture_current(SelectedTextOrigin::ExistingSelection {
            retained_after_copy: true,
        })
    }

    pub fn capture_existing_selection() -> Option<Self> {
        let text = CaretLocator::new().selected_text()?;
        let selected = SelectedText::try_new(text).ok()?;
        Some(Self {
            selected,
            origin: SelectedTextOrigin::ExistingSelection {
                retained_after_copy: true,
            },
            previous_caret_chunk: None,
        })
    }

    pub(super) fn capture_existing_selection_for_replacement(
        intent: SelectionCaptureIntent,
    ) -> Result<Option<Self>, SelectedTextRuntimeError> {
        let ui_selection = CaretLocator::new().selected_text();
        eprintln!(
            "[double-shift] explicit-selection UIA before copy={:?} intent={intent:?}",
            ui_selection
        );

        let origin = SelectedTextOrigin::ExistingSelection {
            retained_after_copy: true,
        };
        if ui_selection.is_none() && intent != SelectionCaptureIntent::UserSelection {
            eprintln!(
                "[double-shift] explicit-selection skipped clipboard probe without UIA selection or user selection intent"
            );
            return Ok(None);
        }

        let (mut session, shortcut) = if let Some(session) =
            Self::capture_current_with_shortcut(origin, CopyShortcut::CtrlInsert)?
        {
            (session, CopyShortcut::CtrlInsert)
        } else if let Some(session) =
            Self::capture_current_with_shortcut(origin, CopyShortcut::CtrlShiftC)?
        {
            (session, CopyShortcut::CtrlShiftC)
        } else if ui_selection.is_some() {
            let Some(session) = Self::capture_current_with_shortcut(origin, CopyShortcut::CtrlC)?
            else {
                eprintln!(
                    "[double-shift] explicit-selection proven by UIA but no copy shortcut produced text"
                );
                return Ok(None);
            };
            (session, CopyShortcut::CtrlC)
        } else {
            eprintln!(
                "[double-shift] explicit-selection intended but safe copy shortcuts produced no text"
            );
            return Ok(None);
        };

        if let Some(ui_selection) = ui_selection.as_deref()
            && session.selected.as_str() != ui_selection
        {
            eprintln!(
                "[double-shift] explicit-selection mismatch UIA={:?} clipboard={:?}",
                ui_selection,
                session.selected.as_str()
            );
            return Err(SelectedTextRuntimeError::SelectionChangedDuringCapture);
        }

        let ui_after_copy = CaretLocator::new().selected_text();
        let retained_after_copy = if ui_after_copy
            .as_deref()
            .is_some_and(|current| current == session.selected.as_str())
        {
            true
        } else if shortcut == CopyShortcut::CtrlInsert {
            Self::selection_retained_after_safe_copy(session.selected.as_str())?
        } else {
            false
        };
        eprintln!(
            "[double-shift] explicit-selection clipboard={:?} shortcut={} UIA after copy={:?} retained_after_copy={retained_after_copy}",
            session.selected.as_str(),
            shortcut.label(),
            ui_after_copy
        );
        session.origin = SelectedTextOrigin::ExistingSelection {
            retained_after_copy,
        };
        Ok(Some(session))
    }

    pub fn capture_previous_word() -> Result<Option<Self>, SelectedTextRuntimeError> {
        ClipboardSnapshot::verify_preservable()?;
        eprintln!("[double-shift] previous-text probe: Shift+Left({PREVIOUS_TEXT_LOOKBACK})");
        inject_backward_selection(PREVIOUS_TEXT_LOOKBACK)?;
        let ui_after_selection = CaretLocator::new().selected_text();
        eprintln!(
            "[double-shift] previous-text UIA after backward selection={:?}; diagnostic only",
            ui_after_selection
        );

        let origin = SelectedTextOrigin::PreviousCaretChunk {
            range: TrailingNonWhitespaceRange {
                leading_chars: 0,
                selected_chars: 0,
                trailing_chars: 0,
            },
            retained_after_copy: false,
        };
        let (mut session, copied, copied_caret_steps, retained_after_copy, source_label) =
            if let Some(session) =
                Self::capture_current_with_shortcut(origin, CopyShortcut::CtrlInsert)?
            {
                let copied = session.selected.as_str().to_owned();
                let copied_caret_steps = caret_navigation_steps(&copied);
                let retained_after_copy =
                    Self::previous_chunk_retained_after_copy(CopyShortcut::CtrlInsert, &copied)?;
                (
                    session,
                    copied,
                    copied_caret_steps,
                    retained_after_copy,
                    CopyShortcut::CtrlInsert.label(),
                )
            } else if let Some(session) =
                Self::capture_current_with_shortcut(origin, CopyShortcut::CtrlShiftC)?
            {
                let copied = session.selected.as_str().to_owned();
                let copied_caret_steps = caret_navigation_steps(&copied);
                let retained_after_copy =
                    Self::previous_chunk_retained_after_copy(CopyShortcut::CtrlShiftC, &copied)?;
                (
                    session,
                    copied,
                    copied_caret_steps,
                    retained_after_copy,
                    CopyShortcut::CtrlShiftC.label(),
                )
            } else {
                eprintln!(
                    "[double-shift] previous-text backward selection produced no copyable text; collapsing any synthetic selection to its right edge"
                );
                inject_key_press(VK_RIGHT_KEY)?;
                return Ok(None);
            };

        if !retained_after_copy {
            inject_key_presses(VK_RIGHT_KEY, copied_caret_steps)?;
        }
        eprintln!(
            "[double-shift] previous-text copied chunk={:?} chars={} caret_steps={} source={} retained_after_copy={} caret_restored={} restore_path={}",
            copied,
            copied.chars().count(),
            copied_caret_steps,
            source_label,
            retained_after_copy,
            !retained_after_copy,
            if retained_after_copy {
                "selection-retained"
            } else {
                "Right"
            }
        );

        let Some(range) = trailing_non_whitespace_range(&copied) else {
            eprintln!("[double-shift] previous-text copied chunk contains only whitespace");
            if retained_after_copy {
                inject_key_press(VK_RIGHT_KEY)?;
            }
            return Ok(None);
        };
        let chars = copied.chars().collect::<Vec<_>>();
        let end = range.leading_chars + range.selected_chars;
        let target = chars[range.leading_chars..end].iter().collect::<String>();
        eprintln!(
            "[double-shift] previous-text range leading={} selected={} trailing={} target={:?}",
            range.leading_chars, range.selected_chars, range.trailing_chars, target
        );
        let selected = SelectedText::try_new(target)
            .map_err(|_| SelectedTextRuntimeError::SelectionChangedDuringCapture)?;

        session.selected = selected;
        session.previous_caret_chunk = Some(copied);
        session.origin = SelectedTextOrigin::PreviousCaretChunk {
            range,
            retained_after_copy,
        };
        Ok(Some(session))
    }

    fn previous_chunk_retained_after_copy(
        shortcut: CopyShortcut,
        copied: &str,
    ) -> Result<bool, SelectedTextRuntimeError> {
        let ui_after_copy = CaretLocator::new().selected_text();
        if ui_after_copy
            .as_deref()
            .is_some_and(|current| current == copied)
        {
            return Ok(true);
        }
        if shortcut == CopyShortcut::CtrlInsert {
            Self::selection_retained_after_safe_copy(copied)
        } else {
            Ok(false)
        }
    }

    fn capture_current(
        origin: SelectedTextOrigin,
    ) -> Result<Option<Self>, SelectedTextRuntimeError> {
        Self::capture_current_with_shortcut(origin, CopyShortcut::CtrlInsert)
    }
    fn capture_current_with_shortcut(
        origin: SelectedTextOrigin,
        shortcut: CopyShortcut,
    ) -> Result<Option<Self>, SelectedTextRuntimeError> {
        let mut snapshot = ClipboardSnapshot::capture()?;
        let _observation_guard = super::clipboard_listener::InternalClipboardMutationGuard::begin();
        let sequence_before_copy = unsafe { GetClipboardSequenceNumber() };
        eprintln!(
            "[double-shift] clipboard capture start origin={origin:?} sequence_before={sequence_before_copy} shortcut={}",
            shortcut.label()
        );

        shortcut.copy_to_clipboard()?;
        if !wait_for_clipboard_change(sequence_before_copy, COPY_TIMEOUT) {
            eprintln!(
                "[double-shift] clipboard capture timeout/no change origin={origin:?} shortcut={}",
                shortcut.label()
            );
            snapshot.restore()?;
            return Ok(None);
        }

        let copied_text = read_unicode_clipboard_text();
        snapshot.restore()?;

        let Some(text) = copied_text? else {
            eprintln!(
                "[double-shift] clipboard changed but contained no Unicode text origin={origin:?} shortcut={}",
                shortcut.label()
            );
            return Ok(None);
        };
        eprintln!(
            "[double-shift] clipboard capture origin={origin:?} shortcut={} text={:?} chars={}",
            shortcut.label(),
            text,
            text.chars().count()
        );
        let Ok(selected) = SelectedText::try_new(text) else {
            eprintln!("[double-shift] clipboard text rejected by SelectedText origin={origin:?}");
            return Ok(None);
        };

        Ok(Some(Self {
            selected,
            origin,
            previous_caret_chunk: None,
        }))
    }

    fn selection_retained_after_safe_copy(
        expected: &str,
    ) -> Result<bool, SelectedTextRuntimeError> {
        let probe = Self::capture_current_with_shortcut(
            SelectedTextOrigin::ExistingSelection {
                retained_after_copy: true,
            },
            CopyShortcut::CtrlInsert,
        )?;
        match probe {
            Some(probe) if probe.selected.as_str() == expected => Ok(true),
            Some(probe) => {
                eprintln!(
                    "[double-shift] retained-selection probe changed text expected={expected:?} actual={:?}",
                    probe.selected.as_str()
                );
                Err(SelectedTextRuntimeError::SelectionChangedDuringCapture)
            }
            None => Ok(false),
        }
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

    pub fn apply_layout_switch(
        self,
        action: &SelectedReplacementAction,
    ) -> Result<(), SelectedTextRuntimeError> {
        if action.source() != &self.selected {
            return Err(SelectedTextRuntimeError::ActionDoesNotMatchSelection);
        }

        eprintln!(
            "[double-shift] apply origin={:?} source={:?} replacement={:?}",
            self.origin,
            self.selected.as_str(),
            action.replacement().as_str()
        );
        match self.origin {
            SelectedTextOrigin::ExistingSelection {
                retained_after_copy: true,
            } => {
                eprintln!("[double-shift] apply path=selection-still-active Ctrl+V paste");
                paste_selected_replacement_preserving_clipboard(action.replacement().as_str())?;
            }
            SelectedTextOrigin::ExistingSelection {
                retained_after_copy: false,
            } => {
                eprintln!(
                    "[double-shift] apply path=selection-collapsed-at-start Shift+Right({})+Delete+Unicode",
                    self.selected.as_str().chars().count()
                );
                inject_current_caret_forward_replacement(
                    self.selected.as_str().chars().count(),
                    action.replacement().as_str(),
                )?;
            }
            SelectedTextOrigin::PreviousCaretChunk {
                range,
                retained_after_copy: true,
            } => {
                let copied = self
                    .previous_caret_chunk
                    .as_deref()
                    .ok_or(SelectedTextRuntimeError::SelectionChangedDuringCapture)?;
                let replacement =
                    replace_copied_chunk_range(copied, range, action.replacement().as_str());
                eprintln!(
                    "[double-shift] apply path=retained-previous-caret-chunk PasteFullSelection chars={}",
                    replacement.chars().count()
                );
                paste_selected_replacement_preserving_clipboard(&replacement)?;
            }
            SelectedTextOrigin::PreviousCaretChunk {
                range,
                retained_after_copy: false,
            } => {
                eprintln!(
                    "[double-shift] apply path=restored-caret Left({})+ShiftLeft({})+replace+Right({})",
                    range.trailing_chars, range.selected_chars, range.trailing_chars
                );
                inject_previous_caret_range_replacement(
                    range.selected_chars,
                    range.trailing_chars,
                    action.replacement().as_str(),
                )?;
            }
        }
        Ok(())
    }

    pub fn finish_without_replacement(self) -> Result<(), SelectedTextRuntimeError> {
        match self.origin {
            SelectedTextOrigin::ExistingSelection {
                retained_after_copy: true,
            } => {}
            SelectedTextOrigin::ExistingSelection {
                retained_after_copy: false,
            } => {
                inject_current_caret_forward_selection(self.selected.as_str().chars().count())?;
            }
            SelectedTextOrigin::PreviousCaretChunk {
                retained_after_copy: true,
                ..
            } => {
                if let Some(copied) = self.previous_caret_chunk.as_deref() {
                    inject_current_caret_forward_selection(caret_navigation_steps(copied))?;
                }
            }
            SelectedTextOrigin::PreviousCaretChunk {
                retained_after_copy: false,
                ..
            } => {}
        }
        Ok(())
    }
}

fn paste_selected_replacement_preserving_clipboard(
    text: &str,
) -> Result<(), SelectedTextRuntimeError> {
    let mut snapshot = ClipboardSnapshot::capture()?;
    let _observation_guard = super::clipboard_listener::InternalClipboardMutationGuard::begin();
    set_unicode_clipboard(text)?;
    inject_ctrl_chord(VK_V_KEY)?;
    sleep(Duration::from_millis(80));
    snapshot.restore()?;
    Ok(())
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
                let Some(text) = read_unicode_clipboard_text()? else {
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

    fn verify_preservable() -> Result<(), SelectedTextRuntimeError> {
        let mut snapshot = Self::capture()?;
        snapshot.restored = true;
        Ok(())
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
            let actual = read_unicode_clipboard_text()?;
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

pub fn read_observable_clipboard_content()
-> Result<Option<ObservableClipboardContent>, SelectedTextRuntimeError> {
    let format_count = unsafe { CountClipboardFormats() };
    let unicode_text_available = unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT as u32) } != 0;
    let file_drop_available = unsafe { IsClipboardFormatAvailable(CF_HDROP as u32) } != 0;
    let image_available = unsafe {
        IsClipboardFormatAvailable(CF_DIBV5 as u32) != 0
            || IsClipboardFormatAvailable(CF_DIB as u32) != 0
    };
    match classify_clipboard_snapshot(
        format_count,
        unicode_text_available,
        file_drop_available,
        image_available,
    ) {
        ClipboardSnapshotMode::UnicodeText => Ok(read_unicode_clipboard_text()?
            .filter(|text| !text.is_empty())
            .map(ObservableClipboardContent::Text)),
        ClipboardSnapshotMode::Image => {
            Ok(read_observable_clipboard_image()?.map(ObservableClipboardContent::Image))
        }
        _ => Ok(None),
    }
}

#[allow(dead_code)]
pub(super) fn read_observable_clipboard_text() -> Result<Option<String>, SelectedTextRuntimeError> {
    let format_count = unsafe { CountClipboardFormats() };
    let unicode_text_available = unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT as u32) } != 0;
    let file_drop_available = unsafe { IsClipboardFormatAvailable(CF_HDROP as u32) } != 0;
    let image_available = unsafe {
        IsClipboardFormatAvailable(CF_DIBV5 as u32) != 0
            || IsClipboardFormatAvailable(CF_DIB as u32) != 0
    };
    if classify_clipboard_snapshot(
        format_count,
        unicode_text_available,
        file_drop_available,
        image_available,
    ) != ClipboardSnapshotMode::UnicodeText
    {
        return Ok(None);
    }
    read_unicode_clipboard_text()
}

fn read_observable_clipboard_image()
-> Result<Option<ClipboardImagePayload>, SelectedTextRuntimeError> {
    let (format, name) = unsafe {
        if IsClipboardFormatAvailable(CF_DIBV5 as u32) != 0 {
            (CF_DIBV5 as u32, "CF_DIBV5")
        } else if IsClipboardFormatAvailable(CF_DIB as u32) != 0 {
            (CF_DIB as u32, "CF_DIB")
        } else {
            return Ok(None);
        }
    };
    let _clipboard = ClipboardOpenGuard::open()?;
    let bytes = read_open_hglobal_bytes(format)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    Ok(Some(ClipboardImagePayload::new(name.to_owned(), bytes)))
}

fn read_unicode_clipboard_text() -> Result<Option<String>, SelectedTextRuntimeError> {
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

pub(super) fn set_unicode_clipboard(text: &str) -> Result<(), SelectedTextRuntimeError> {
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

pub(super) fn set_image_clipboard_payload(
    format_name: &str,
    data: &[u8],
) -> Result<(), SelectedTextRuntimeError> {
    if data.is_empty() {
        return Err(SelectedTextRuntimeError::ClipboardWriteFailed);
    }
    let format = match format_name {
        "CF_DIBV5" => CF_DIBV5 as u32,
        "CF_DIB" => CF_DIB as u32,
        _ => return Err(SelectedTextRuntimeError::UnsupportedClipboardState),
    };
    let clipboard = ClipboardOpenGuard::open()?;
    if unsafe { EmptyClipboard() } == 0 {
        return Err(SelectedTextRuntimeError::ClipboardWriteFailed);
    }
    set_open_hglobal_bytes(format, data)?;
    drop(clipboard);
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
