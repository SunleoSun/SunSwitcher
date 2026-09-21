use std::mem::{size_of, zeroed};
use std::ptr::null_mut;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, GetKeyboardLayout, INPUT, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, SendInput, ToUnicodeEx, VK_BACK, VK_CAPITAL, VK_CONTROL,
    VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LMENU,
    VK_LSHIFT, VK_LWIN, VK_MENU, VK_NEXT, VK_NUMLOCK, VK_OEM_1, VK_OEM_3, VK_OEM_4, VK_OEM_6,
    VK_OEM_7, VK_OEM_COMMA, VK_OEM_PERIOD, VK_PAUSE, VK_PRIOR, VK_RCONTROL, VK_RETURN, VK_RIGHT,
    VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SCROLL, VK_SHIFT, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId,
    HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL,
    WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_RBUTTONDOWN, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WM_USER, WM_XBUTTONDOWN,
};

use crate::input::{Boundary, InputEvent, PhysicalKey, TypedCharacter};
use crate::persistence::UndoHotkey;
use crate::replacement::{ReplacementAction, ReplacementOutcome, UndoOutcome};

const SUNSWITCHER_INJECTED_MARKER: usize = 0x5355_4E53_5749_5443;
const TO_UNICODE_DO_NOT_CHANGE_STATE: u32 = 0x4;

pub trait InputProcessor: Send + 'static {
    fn process(&mut self, event: InputEvent) -> RuntimeDirective;

    fn replacement_outcome(&mut self, _outcome: ReplacementOutcome) {}

    fn undo(&mut self) -> UndoDirective {
        UndoDirective::Pass
    }

    fn undo_outcome(&mut self, _outcome: UndoOutcome) {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeDirective {
    Pass,
    Replace(ReplacementAction),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoDirective {
    Pass,
    Restore(ReplacementAction),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    AlreadyRunning,
    RuntimeStateUnavailable,
    HookInstallFailed,
    MouseHookInstallFailed,
    HookUninstallFailed,
    MouseHookUninstallFailed,
    MessageLoopFailed,
    InjectionFailed { expected: u32, sent: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct InputOwnershipStamp {
    input_revision: u64,
    foreground_window_id: usize,
}

impl InputOwnershipStamp {
    pub(super) const fn new(input_revision: u64, foreground_window_id: usize) -> Self {
        Self {
            input_revision,
            foreground_window_id,
        }
    }
}

struct RuntimeState {
    processor: Box<dyn InputProcessor>,
    keyboard_state: [u8; 256],
    suppressed_keyup_vk: Option<u32>,
    foreground_window_id: usize,
    undo_hotkey: UndoHotkey,
    input_revision: u64,
}

impl RuntimeState {
    fn new(mut processor: Box<dyn InputProcessor>, undo_hotkey: UndoHotkey) -> Self {
        // The runtime cannot prove where the caret is when a hook is attached. Start fail-closed;
        // the input owner may resume tracking after it observes an unambiguous boundary.
        let _ = processor.process(InputEvent::Invalidate);
        Self {
            processor,
            keyboard_state: initial_keyboard_state(),
            suppressed_keyup_vk: None,
            foreground_window_id: current_foreground_window_id(),
            undo_hotkey,
            input_revision: 0,
        }
    }

    fn sync_foreground_window(&mut self, current: usize) -> bool {
        let changed = foreground_change_requires_invalidation(self.foreground_window_id, current);
        self.foreground_window_id = current;
        changed
    }

    fn update_key_state(&mut self, vk_code: u32, is_key_down: bool) {
        update_keyboard_state(&mut self.keyboard_state, vk_code, is_key_down);
    }

    fn command_modifier_active(&self) -> bool {
        self.keyboard_state[VK_CONTROL as usize] & 0x80 != 0
            || self.keyboard_state[VK_MENU as usize] & 0x80 != 0
            || self.keyboard_state[VK_LWIN as usize] & 0x80 != 0
            || self.keyboard_state[VK_RWIN as usize] & 0x80 != 0
    }

    fn undo_hotkey_matches(&self, vk_code: u32) -> bool {
        undo_hotkey_matches(self.undo_hotkey, vk_code, &self.keyboard_state)
    }

    fn note_external_input(&mut self) {
        self.input_revision = self.input_revision.wrapping_add(1);
    }

    const fn ownership_stamp(&self) -> InputOwnershipStamp {
        InputOwnershipStamp::new(self.input_revision, self.foreground_window_id)
    }
}

static RUNTIME: Mutex<Option<RuntimeState>> = Mutex::new(None);
static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);
static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
const WM_SUNSWITCHER_STOP: u32 = WM_USER + 0x535;

struct RuntimeRegistration;

impl RuntimeRegistration {
    fn install(
        processor: impl InputProcessor,
        undo_hotkey: UndoHotkey,
    ) -> Result<Self, RuntimeError> {
        let mut runtime = RUNTIME
            .lock()
            .map_err(|_| RuntimeError::RuntimeStateUnavailable)?;
        if runtime.is_some() {
            return Err(RuntimeError::AlreadyRunning);
        }
        *runtime = Some(RuntimeState::new(Box::new(processor), undo_hotkey));
        Ok(Self)
    }
}

impl Drop for RuntimeRegistration {
    fn drop(&mut self) {
        HOOK_THREAD_ID.store(0, Ordering::Release);
        STOP_REQUESTED.store(false, Ordering::Release);
        if let Ok(mut runtime) = RUNTIME.lock() {
            *runtime = None;
        }
    }
}

struct HookGuard(HHOOK);

impl HookGuard {
    fn new(hook: HHOOK) -> Self {
        Self(hook)
    }

    fn release(&mut self) -> bool {
        if self.0.is_null() {
            return true;
        }
        if unsafe { UnhookWindowsHookEx(self.0) } == 0 {
            return false;
        }
        self.0 = null_mut();
        true
    }
}

impl Drop for HookGuard {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

fn current_foreground_window_id() -> usize {
    unsafe { GetForegroundWindow() as usize }
}

pub(super) fn foreground_change_requires_invalidation(previous: usize, current: usize) -> bool {
    previous != current
}

fn initial_keyboard_state() -> [u8; 256] {
    let mut state = [0u8; 256];
    for (vk_code, slot) in state.iter_mut().enumerate() {
        let async_state = unsafe { GetAsyncKeyState(vk_code as i32) };
        if async_state < 0 {
            *slot |= 0x80;
        }
        if is_toggle_key(vk_code as u32) {
            let toggle_state = unsafe { GetKeyState(vk_code as i32) };
            if toggle_state & 1 != 0 {
                *slot |= 0x01;
            }
        }
    }
    state
}

pub(super) fn is_toggle_key(vk_code: u32) -> bool {
    [VK_CAPITAL, VK_NUMLOCK, VK_SCROLL]
        .into_iter()
        .any(|key| vk_code == key as u32)
}

pub(super) fn update_keyboard_state(state: &mut [u8; 256], vk_code: u32, is_key_down: bool) {
    // Low-level physical events normally identify left/right modifiers, while automation may emit
    // only VK_SHIFT/VK_CONTROL/VK_MENU. Internally treat a generic transition as the left side so
    // it survives unrelated key events until the matching generic key-up arrives.
    let tracked_vk = match vk_code as u16 {
        VK_SHIFT => VK_LSHIFT as u32,
        VK_CONTROL => VK_LCONTROL as u32,
        VK_MENU => VK_LMENU as u32,
        _ => vk_code,
    };

    let Ok(index) = usize::try_from(tracked_vk) else {
        return;
    };
    if index >= state.len() {
        return;
    }

    let was_down = state[index] & 0x80 != 0;
    if is_key_down {
        if !was_down && is_toggle_key(tracked_vk) {
            state[index] ^= 0x01;
        }
        state[index] |= 0x80;
    } else {
        state[index] &= 0x7f;
    }

    sync_generic_modifier(state, VK_SHIFT, VK_LSHIFT, VK_RSHIFT);
    sync_generic_modifier(state, VK_CONTROL, VK_LCONTROL, VK_RCONTROL);
    sync_generic_modifier(state, VK_MENU, VK_LMENU, VK_RMENU);
}

fn sync_generic_modifier(state: &mut [u8; 256], generic: u16, left: u16, right: u16) {
    let active = state[left as usize] & 0x80 != 0 || state[right as usize] & 0x80 != 0;
    if active {
        state[generic as usize] |= 0x80;
    } else {
        state[generic as usize] &= 0x7f;
    }
}

pub fn run_global_keyboard_hook(
    processor: impl InputProcessor,
    undo_hotkey: UndoHotkey,
) -> Result<(), RuntimeError> {
    let _runtime_registration = RuntimeRegistration::install(processor, undo_hotkey)?;

    unsafe {
        let keyboard_hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), null_mut(), 0);
        if keyboard_hook.is_null() {
            return Err(RuntimeError::HookInstallFailed);
        }
        let mut keyboard_hook = HookGuard::new(keyboard_hook);

        let mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), null_mut(), 0);
        if mouse_hook.is_null() {
            return if keyboard_hook.release() {
                Err(RuntimeError::MouseHookInstallFailed)
            } else {
                Err(RuntimeError::HookUninstallFailed)
            };
        }
        let mut mouse_hook = HookGuard::new(mouse_hook);

        let mut message: MSG = zeroed();
        PeekMessageW(&mut message, null_mut(), WM_USER, WM_USER, PM_NOREMOVE);
        HOOK_THREAD_ID.store(GetCurrentThreadId(), Ordering::Release);

        let loop_result = if STOP_REQUESTED.load(Ordering::Acquire) {
            Ok(())
        } else {
            loop {
                let result = GetMessageW(&mut message, null_mut(), 0, 0);
                if result == -1 {
                    break Err(RuntimeError::MessageLoopFailed);
                }
                if result == 0 || message.message == WM_SUNSWITCHER_STOP {
                    break Ok(());
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        };

        let mouse_released = mouse_hook.release();
        let keyboard_released = keyboard_hook.release();
        if !mouse_released {
            return Err(RuntimeError::MouseHookUninstallFailed);
        }
        if !keyboard_released {
            return Err(RuntimeError::HookUninstallFailed);
        }
        loop_result
    }
}

pub fn request_global_keyboard_hook_stop() -> bool {
    let thread_id = HOOK_THREAD_ID.load(Ordering::Acquire);
    if thread_id == 0 {
        let running = RUNTIME
            .lock()
            .map(|runtime| runtime.is_some())
            .unwrap_or(false);
        if !running {
            return false;
        }
        STOP_REQUESTED.store(true, Ordering::Release);
        return true;
    }

    STOP_REQUESTED.store(true, Ordering::Release);
    unsafe { PostThreadMessageW(thread_id, WM_SUNSWITCHER_STOP, 0, 0) != 0 }
}

unsafe extern "system" fn keyboard_hook(code: i32, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
    if code < 0 {
        return unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
    }

    let event = unsafe { &*(l_param as *const KBDLLHOOKSTRUCT) };
    if event.dwExtraInfo == SUNSWITCHER_INJECTED_MARKER {
        return unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
    }

    let message = w_param as u32;
    let is_key_down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
    let is_key_up = message == WM_KEYUP || message == WM_SYSKEYUP;
    if !is_key_down && !is_key_up {
        return unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
    }

    if is_foreign_injected_keyboard_event(event.flags, event.dwExtraInfo) {
        // Ask downstream hooks first: if one suppresses the injected transition, it must not
        // become part of our modifier/toggle state. The attempted foreign edit still invalidates
        // owned text either way.
        let next_result = unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
        if let Ok(mut runtime_slot) = RUNTIME.lock()
            && let Some(runtime) = runtime_slot.as_mut()
        {
            runtime.note_external_input();
            if next_result == 0 {
                runtime.update_key_state(event.vkCode, is_key_down);
            }
            let _ = runtime.processor.process(InputEvent::Invalidate);
        }
        return next_result;
    }

    let state = {
        let Ok(mut runtime_slot) = RUNTIME.lock() else {
            return unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
        };
        let Some(runtime) = runtime_slot.as_mut() else {
            return unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
        };

        runtime.note_external_input();
        if is_key_down {
            let foreground_window_id = current_foreground_window_id();
            if runtime.sync_foreground_window(foreground_window_id) {
                let _ = runtime.processor.process(InputEvent::Invalidate);
            }
        }

        runtime.update_key_state(event.vkCode, is_key_down);
        let ownership_stamp = runtime.ownership_stamp();

        if is_key_up {
            let suppress = runtime.suppressed_keyup_vk == Some(event.vkCode);
            if suppress {
                runtime.suppressed_keyup_vk = None;
            }
            HookEventState::KeyUp { suppress }
        } else if runtime.undo_hotkey_matches(event.vkCode) {
            HookEventState::UndoHotkey { ownership_stamp }
        } else {
            let directive = classify_key_down(event, runtime)
                .map(|input_event| runtime.processor.process(input_event))
                .unwrap_or(RuntimeDirective::Pass);
            match directive {
                RuntimeDirective::Pass => HookEventState::KeyDownPass,
                RuntimeDirective::Replace(action) => HookEventState::KeyDownReplace {
                    action,
                    ownership_stamp,
                },
            }
        }
    };

    match state {
        HookEventState::KeyUp { suppress } => {
            let next_result = unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
            if suppress { 1 } else { next_result }
        }
        HookEventState::KeyDownPass => {
            let next_result = unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
            if next_result != 0 {
                invalidate_runtime_tracking();
            }
            next_result
        }
        HookEventState::UndoHotkey { ownership_stamp } => {
            let next_result = unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
            if next_result != 0 {
                invalidate_runtime_tracking();
                return next_result;
            }
            if !runtime_input_ownership_unchanged(ownership_stamp) {
                invalidate_runtime_tracking();
                return 1;
            }

            // Resolve the Undo only after downstream hooks return. A downstream hook may inject
            // input re-entrantly or change the foreground window while handling Pause; those
            // changes must disarm the immediate Undo before we decide it is still safe to execute.
            let directive = RUNTIME
                .lock()
                .ok()
                .and_then(|mut runtime_slot| {
                    runtime_slot
                        .as_mut()
                        .map(|runtime| runtime.processor.undo())
                })
                .unwrap_or(UndoDirective::Pass);
            if let UndoDirective::Restore(action) = directive {
                let injection_result = inject_replacement(&action);
                let ownership_unchanged = runtime_input_ownership_unchanged(ownership_stamp);
                let outcome = undo_outcome_after_injection(&injection_result, ownership_unchanged);
                notify_runtime_undo_outcome(outcome);
                if outcome == UndoOutcome::Uncertain {
                    invalidate_runtime_tracking();
                }
                if let Err(error) = injection_result {
                    eprintln!("SunSwitcher undo injection failed: {error:?}");
                }
            }
            if let Ok(mut runtime_slot) = RUNTIME.lock()
                && let Some(runtime) = runtime_slot.as_mut()
            {
                runtime.suppressed_keyup_vk = Some(event.vkCode);
            }
            1
        }
        HookEventState::KeyDownReplace {
            action,
            ownership_stamp,
        } => {
            // Let downstream hooks observe the real physical event before SunSwitcher suppresses
            // it from the target application. This keeps other low-level hook state machines in
            // sync instead of starving them of the boundary key that triggered replacement.
            let next_result = unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
            if next_result != 0 {
                // A downstream hook suppressed the boundary that completed our token, so the
                // application's visible caret/text state no longer has the boundary we assumed.
                // Stay desynchronized until a later unambiguous boundary reaches the stream.
                notify_runtime_replacement_outcome(ReplacementOutcome::Aborted);
                invalidate_runtime_tracking();
                return next_result;
            }
            if !runtime_input_ownership_unchanged(ownership_stamp) {
                // Downstream hook work re-entered SunSwitcher and changed observable input/caret
                // ownership. The old action can no longer safely delete text at the current caret.
                notify_runtime_replacement_outcome(ReplacementOutcome::Aborted);
                invalidate_runtime_tracking();
                return next_result;
            }

            let injection_result = inject_replacement(&action);
            let ownership_unchanged = runtime_input_ownership_unchanged(ownership_stamp);
            let replacement_outcome =
                replacement_outcome_after_injection(&injection_result, ownership_unchanged);
            if let Some(vk_code) =
                keyup_suppression_after_injection(event.vkCode, &injection_result)
            {
                if let Ok(mut runtime_slot) = RUNTIME.lock()
                    && let Some(runtime) = runtime_slot.as_mut()
                {
                    runtime.suppressed_keyup_vk = Some(vk_code);
                    runtime.processor.replacement_outcome(replacement_outcome);
                    if replacement_outcome == ReplacementOutcome::Aborted {
                        let _ = runtime.processor.process(InputEvent::Invalidate);
                    }
                }
                return 1;
            }

            notify_runtime_replacement_outcome(replacement_outcome);
            if replacement_outcome == ReplacementOutcome::Aborted {
                invalidate_runtime_tracking();
            }
            if let Err(error) = injection_result {
                eprintln!("SunSwitcher replacement injection failed: {error:?}");
            }
            next_result
        }
    }
}

#[derive(Debug)]
enum HookEventState {
    KeyDownPass,
    KeyDownReplace {
        action: ReplacementAction,
        ownership_stamp: InputOwnershipStamp,
    },
    UndoHotkey {
        ownership_stamp: InputOwnershipStamp,
    },
    KeyUp {
        suppress: bool,
    },
}

fn notify_runtime_undo_outcome(outcome: UndoOutcome) {
    if let Ok(mut runtime_slot) = RUNTIME.lock()
        && let Some(runtime) = runtime_slot.as_mut()
    {
        runtime.processor.undo_outcome(outcome);
    }
}

fn notify_runtime_replacement_outcome(outcome: ReplacementOutcome) {
    if let Ok(mut runtime_slot) = RUNTIME.lock()
        && let Some(runtime) = runtime_slot.as_mut()
    {
        runtime.processor.replacement_outcome(outcome);
    }
}

fn invalidate_runtime_tracking() {
    if let Ok(mut runtime_slot) = RUNTIME.lock()
        && let Some(runtime) = runtime_slot.as_mut()
    {
        let _ = runtime.processor.process(InputEvent::Invalidate);
    }
}

fn runtime_input_ownership_unchanged(expected: InputOwnershipStamp) -> bool {
    let current_foreground_window_id = current_foreground_window_id();
    RUNTIME
        .lock()
        .ok()
        .and_then(|runtime_slot| {
            runtime_slot.as_ref().map(|runtime| {
                input_ownership_matches(
                    expected,
                    runtime.input_revision,
                    runtime.foreground_window_id,
                    current_foreground_window_id,
                )
            })
        })
        .unwrap_or(false)
}

pub(super) fn input_ownership_matches(
    expected: InputOwnershipStamp,
    current_revision: u64,
    tracked_foreground_window_id: usize,
    current_foreground_window_id: usize,
) -> bool {
    current_revision == expected.input_revision
        && tracked_foreground_window_id == expected.foreground_window_id
        && current_foreground_window_id == expected.foreground_window_id
}

pub(super) fn undo_hotkey_matches(
    hotkey: UndoHotkey,
    vk_code: u32,
    keyboard_state: &[u8; 256],
) -> bool {
    let no_modifiers = [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
        .into_iter()
        .all(|key| keyboard_state[key as usize] & 0x80 == 0);
    match hotkey {
        UndoHotkey::Pause => vk_code == VK_PAUSE as u32 && no_modifiers,
    }
}

pub(super) fn is_foreign_injected_keyboard_event(flags: u32, extra_info: usize) -> bool {
    flags & LLKHF_INJECTED != 0 && extra_info != SUNSWITCHER_INJECTED_MARKER
}

unsafe extern "system" fn mouse_hook_proc(code: i32, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
    if code >= 0
        && mouse_message_invalidates_tracking(w_param as u32)
        && let Ok(mut runtime_slot) = RUNTIME.lock()
        && let Some(runtime) = runtime_slot.as_mut()
    {
        runtime.note_external_input();
        let _ = runtime.processor.process(InputEvent::Invalidate);
    }
    unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) }
}

pub(super) fn mouse_message_invalidates_tracking(message: u32) -> bool {
    matches!(
        message,
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KeyDownDisposition {
    Ignore,
    Event(InputEvent),
    Translate,
}

pub(super) fn preclassify_key_down(
    vk_code: u32,
    command_modifier_active: bool,
) -> KeyDownDisposition {
    // Shift is part of ordinary text entry. It must not invalidate an already tracked token,
    // otherwise shifted punctuation (for example "!") would erase the token just before its
    // boundary arrives.
    if is_shift_modifier_key(vk_code) {
        return KeyDownDisposition::Ignore;
    }

    // Command-modified editing keys have application-specific semantics (Ctrl+Backspace,
    // Ctrl+Enter, Ctrl+Tab, Alt+..., Win+...). They must invalidate tracked text before any
    // special-key interpretation so SunSwitcher never reinterprets them as ordinary editing.
    if command_modifier_active {
        return KeyDownDisposition::Event(InputEvent::Invalidate);
    }
    if vk_code == VK_BACK as u32 {
        return KeyDownDisposition::Event(InputEvent::Backspace);
    }
    if vk_code == VK_RETURN as u32 {
        return KeyDownDisposition::Event(InputEvent::Boundary(Boundary::Enter));
    }
    if vk_code == VK_TAB as u32 {
        return KeyDownDisposition::Event(InputEvent::Boundary(Boundary::Tab));
    }
    if is_state_invalidating_key(vk_code) {
        return KeyDownDisposition::Event(InputEvent::Invalidate);
    }

    KeyDownDisposition::Translate
}

fn classify_key_down(event: &KBDLLHOOKSTRUCT, runtime: &RuntimeState) -> Option<InputEvent> {
    match preclassify_key_down(event.vkCode, runtime.command_modifier_active()) {
        KeyDownDisposition::Ignore => None,
        KeyDownDisposition::Event(event) => Some(event),
        KeyDownDisposition::Translate => Some(
            translate_key(event, &runtime.keyboard_state)
                .map(InputEvent::Character)
                .unwrap_or(InputEvent::Invalidate),
        ),
    }
}

pub(super) fn is_shift_modifier_key(vk_code: u32) -> bool {
    [VK_SHIFT, VK_LSHIFT, VK_RSHIFT]
        .into_iter()
        .any(|key| vk_code == key as u32)
}

fn is_state_invalidating_key(vk_code: u32) -> bool {
    [
        VK_ESCAPE, VK_DELETE, VK_INSERT, VK_HOME, VK_END, VK_PRIOR, VK_NEXT, VK_LEFT, VK_RIGHT,
        VK_UP, VK_DOWN,
    ]
    .into_iter()
    .any(|key| vk_code == key as u32)
}

fn translate_key(event: &KBDLLHOOKSTRUCT, keyboard_state: &[u8; 256]) -> Option<TypedCharacter> {
    let foreground = unsafe { GetForegroundWindow() };
    let thread_id = if foreground.is_null() {
        0
    } else {
        unsafe { GetWindowThreadProcessId(foreground, null_mut()) }
    };
    let layout = unsafe { GetKeyboardLayout(thread_id) };
    let mut utf16 = [0u16; 8];
    let written = unsafe {
        ToUnicodeEx(
            event.vkCode,
            event.scanCode,
            keyboard_state.as_ptr(),
            utf16.as_mut_ptr(),
            utf16.len() as i32,
            TO_UNICODE_DO_NOT_CHANGE_STATE,
            layout,
        )
    };
    if written <= 0 {
        return None;
    }

    let decoded: Vec<char> = char::decode_utf16(utf16[..written as usize].iter().copied())
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (decoded.len() == 1)
        .then(|| TypedCharacter::new(decoded[0], physical_key_from_vk(event.vkCode)))
}

pub(super) fn physical_key_from_vk(vk_code: u32) -> PhysicalKey {
    match vk_code as u16 {
        VK_OEM_3 => PhysicalKey::Grave,
        VK_OEM_4 => PhysicalKey::LeftBracket,
        VK_OEM_6 => PhysicalKey::RightBracket,
        VK_OEM_1 => PhysicalKey::Semicolon,
        VK_OEM_7 => PhysicalKey::Quote,
        VK_OEM_COMMA => PhysicalKey::Comma,
        VK_OEM_PERIOD => PhysicalKey::Period,
        _ => PhysicalKey::Other,
    }
}

fn inject_replacement(action: &ReplacementAction) -> Result<(), RuntimeError> {
    send_inputs(&build_replacement_inputs(action))
}

pub(super) fn undo_outcome_after_injection(
    injection_result: &Result<(), RuntimeError>,
    state_unchanged: bool,
) -> UndoOutcome {
    if !state_unchanged {
        return UndoOutcome::Uncertain;
    }
    match injection_result {
        Ok(()) => UndoOutcome::Applied,
        Err(RuntimeError::InjectionFailed { sent: 0, .. }) => UndoOutcome::NotExecuted,
        Err(_) => UndoOutcome::Uncertain,
    }
}

pub(super) fn replacement_outcome_after_injection(
    injection_result: &Result<(), RuntimeError>,
    state_unchanged: bool,
) -> ReplacementOutcome {
    if injection_result.is_ok() && state_unchanged {
        ReplacementOutcome::Applied
    } else {
        ReplacementOutcome::Aborted
    }
}

pub(super) fn keyup_suppression_after_injection(
    vk_code: u32,
    injection_result: &Result<(), RuntimeError>,
) -> Option<u32> {
    injection_result.as_ref().ok().map(|_| vk_code)
}

pub(super) fn inject_selected_text(text: &str) -> Result<(), RuntimeError> {
    send_inputs(&build_selected_text_inputs(text))
}

pub(super) fn build_selected_text_inputs(text: &str) -> Vec<INPUT> {
    if text.is_empty() {
        return vec![
            virtual_key_input(VK_DELETE, false),
            virtual_key_input(VK_DELETE, true),
        ];
    }

    let mut inputs = Vec::with_capacity(text.encode_utf16().count() * 2);
    for unit in text.encode_utf16() {
        inputs.push(unicode_input(unit, false));
        inputs.push(unicode_input(unit, true));
    }
    inputs
}

pub(super) fn inject_ctrl_chord(key: u16) -> Result<(), RuntimeError> {
    send_inputs(&build_ctrl_chord_inputs(key))
}

pub(super) fn build_ctrl_chord_inputs(key: u16) -> Vec<INPUT> {
    vec![
        virtual_key_input(VK_CONTROL, false),
        virtual_key_input(key, false),
        virtual_key_input(key, true),
        virtual_key_input(VK_CONTROL, true),
    ]
}

fn send_inputs(inputs: &[INPUT]) -> Result<(), RuntimeError> {
    let expected = inputs.len() as u32;
    let sent = unsafe { SendInput(expected, inputs.as_ptr(), size_of::<INPUT>() as i32) };
    if sent != expected {
        return Err(RuntimeError::InjectionFailed { expected, sent });
    }
    Ok(())
}

pub(super) fn build_replacement_inputs(action: &ReplacementAction) -> Vec<INPUT> {
    let mut inputs = Vec::new();
    for _ in 0..action.delete_previous_chars() {
        inputs.push(virtual_key_input(VK_BACK, false));
        inputs.push(virtual_key_input(VK_BACK, true));
    }
    for unit in action.replacement().as_str().encode_utf16() {
        inputs.push(unicode_input(unit, false));
        inputs.push(unicode_input(unit, true));
    }
    match action.boundary() {
        Boundary::Character(character) => {
            for unit in character.encode_utf16(&mut [0u16; 2]).iter().copied() {
                inputs.push(unicode_input(unit, false));
                inputs.push(unicode_input(unit, true));
            }
        }
        Boundary::Enter => {
            inputs.push(virtual_key_input(VK_RETURN, false));
            inputs.push(virtual_key_input(VK_RETURN, true));
        }
        Boundary::Tab => {
            // The physical Tab is still held while the hook suppresses its original key-down.
            // Normalize the global key state with an injected key-up before replaying a full
            // Tab press; otherwise some controls ignore the replayed key-down as a repeat.
            inputs.push(virtual_key_input(VK_TAB, true));
            inputs.push(virtual_key_input(VK_TAB, false));
            inputs.push(virtual_key_input(VK_TAB, true));
        }
    }
    inputs
}

fn virtual_key_input(key: u16, key_up: bool) -> INPUT {
    let mut input: INPUT = unsafe { zeroed() };
    input.r#type = INPUT_KEYBOARD;
    input.Anonymous.ki = KEYBDINPUT {
        wVk: key,
        wScan: 0,
        dwFlags: if key_up { KEYEVENTF_KEYUP } else { 0 },
        time: 0,
        dwExtraInfo: SUNSWITCHER_INJECTED_MARKER,
    };
    input
}

fn unicode_input(unit: u16, key_up: bool) -> INPUT {
    let mut input: INPUT = unsafe { zeroed() };
    input.r#type = INPUT_KEYBOARD;
    input.Anonymous.ki = KEYBDINPUT {
        wVk: 0,
        wScan: unit,
        dwFlags: KEYEVENTF_UNICODE | if key_up { KEYEVENTF_KEYUP } else { 0 },
        time: 0,
        dwExtraInfo: SUNSWITCHER_INJECTED_MARKER,
    };
    input
}

#[cfg(test)]
pub(super) fn injected_marker() -> usize {
    SUNSWITCHER_INJECTED_MARKER
}
