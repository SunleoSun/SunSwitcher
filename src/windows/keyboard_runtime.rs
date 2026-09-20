use std::mem::{size_of, zeroed};
use std::ptr::null_mut;
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, GetKeyboardLayout, INPUT, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, SendInput, ToUnicodeEx, VK_BACK, VK_CAPITAL, VK_CONTROL,
    VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LMENU,
    VK_LSHIFT, VK_LWIN, VK_MENU, VK_NEXT, VK_NUMLOCK, VK_PRIOR, VK_RCONTROL, VK_RETURN, VK_RIGHT,
    VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SCROLL, VK_SHIFT, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId,
    KBDLLHOOKSTRUCT, MSG, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL,
    WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_RBUTTONDOWN,
    WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN,
};

use crate::input::{Boundary, InputEvent};
use crate::replacement::ReplacementAction;

const SUNSWITCHER_INJECTED_MARKER: usize = 0x5355_4E53_5749_5443;
const TO_UNICODE_DO_NOT_CHANGE_STATE: u32 = 0x4;

pub trait InputProcessor: Send + 'static {
    fn process(&mut self, event: InputEvent) -> RuntimeDirective;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeDirective {
    Pass,
    Replace(ReplacementAction),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    AlreadyRunning,
    HookInstallFailed,
    MouseHookInstallFailed,
    MessageLoopFailed,
    InjectionFailed { expected: u32, sent: u32 },
}

struct RuntimeState {
    processor: Box<dyn InputProcessor>,
    keyboard_state: [u8; 256],
    suppressed_keyup_vk: Option<u32>,
    foreground_window_id: usize,
}

impl RuntimeState {
    fn new(processor: Box<dyn InputProcessor>) -> Self {
        Self {
            processor,
            keyboard_state: initial_keyboard_state(),
            suppressed_keyup_vk: None,
            foreground_window_id: current_foreground_window_id(),
        }
    }

    fn sync_foreground_window(&mut self, current: usize) -> bool {
        let changed = foreground_change_requires_invalidation(self.foreground_window_id, current);
        self.foreground_window_id = current;
        changed
    }

    fn update_key_state(&mut self, vk_code: u32, is_key_down: bool) {
        let Ok(index) = usize::try_from(vk_code) else {
            return;
        };
        if index >= self.keyboard_state.len() {
            return;
        }
        let was_down = self.keyboard_state[index] & 0x80 != 0;
        if is_key_down {
            if !was_down && is_toggle_key(vk_code) {
                self.keyboard_state[index] ^= 0x01;
            }
            self.keyboard_state[index] |= 0x80;
        } else {
            self.keyboard_state[index] &= 0x7f;
        }
        self.sync_generic_modifier(VK_SHIFT, VK_LSHIFT, VK_RSHIFT);
        self.sync_generic_modifier(VK_CONTROL, VK_LCONTROL, VK_RCONTROL);
        self.sync_generic_modifier(VK_MENU, VK_LMENU, VK_RMENU);
    }

    fn sync_generic_modifier(&mut self, generic: u16, left: u16, right: u16) {
        let active = self.keyboard_state[left as usize] & 0x80 != 0
            || self.keyboard_state[right as usize] & 0x80 != 0;
        if active {
            self.keyboard_state[generic as usize] |= 0x80;
        } else {
            self.keyboard_state[generic as usize] &= 0x7f;
        }
    }

    fn command_modifier_active(&self) -> bool {
        self.keyboard_state[VK_CONTROL as usize] & 0x80 != 0
            || self.keyboard_state[VK_MENU as usize] & 0x80 != 0
            || self.keyboard_state[VK_LWIN as usize] & 0x80 != 0
            || self.keyboard_state[VK_RWIN as usize] & 0x80 != 0
    }
}

static RUNTIME: OnceLock<Mutex<RuntimeState>> = OnceLock::new();

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

pub fn run_global_keyboard_hook(processor: impl InputProcessor) -> Result<(), RuntimeError> {
    RUNTIME
        .set(Mutex::new(RuntimeState::new(Box::new(processor))))
        .map_err(|_| RuntimeError::AlreadyRunning)?;

    unsafe {
        let keyboard_hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), null_mut(), 0);
        if keyboard_hook.is_null() {
            return Err(RuntimeError::HookInstallFailed);
        }
        let mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), null_mut(), 0);
        if mouse_hook.is_null() {
            UnhookWindowsHookEx(keyboard_hook);
            return Err(RuntimeError::MouseHookInstallFailed);
        }

        let mut message: MSG = zeroed();
        loop {
            let result = GetMessageW(&mut message, null_mut(), 0, 0);
            if result == -1 {
                UnhookWindowsHookEx(mouse_hook);
                UnhookWindowsHookEx(keyboard_hook);
                return Err(RuntimeError::MessageLoopFailed);
            }
            if result == 0 {
                break;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        UnhookWindowsHookEx(mouse_hook);
        UnhookWindowsHookEx(keyboard_hook);
    }

    Ok(())
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

    let Some(runtime) = RUNTIME.get() else {
        return unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
    };
    let Ok(mut runtime) = runtime.lock() else {
        return unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
    };

    if is_key_down {
        let foreground_window_id = current_foreground_window_id();
        if runtime.sync_foreground_window(foreground_window_id) {
            let _ = runtime.processor.process(InputEvent::Invalidate);
        }
    }

    runtime.update_key_state(event.vkCode, is_key_down);

    if is_key_up {
        if runtime.suppressed_keyup_vk == Some(event.vkCode) {
            runtime.suppressed_keyup_vk = None;
            return 1;
        }
        return unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
    }

    let Some(input_event) = classify_key_down(event, &runtime) else {
        return unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
    };
    let directive = runtime.processor.process(input_event);
    match directive {
        RuntimeDirective::Pass => unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) },
        RuntimeDirective::Replace(action) => {
            let injection_result = inject_replacement(&action);
            if let Some(vk_code) =
                keyup_suppression_after_injection(event.vkCode, &injection_result)
            {
                runtime.suppressed_keyup_vk = Some(vk_code);
                return 1;
            }

            if let Err(error) = injection_result {
                eprintln!("SunSwitcher replacement injection failed: {error:?}");
            }
            unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) }
        }
    }
}

unsafe extern "system" fn mouse_hook_proc(code: i32, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
    if code >= 0
        && mouse_message_invalidates_tracking(w_param as u32)
        && let Some(runtime) = RUNTIME.get()
        && let Ok(mut runtime) = runtime.lock()
    {
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

fn translate_key(event: &KBDLLHOOKSTRUCT, keyboard_state: &[u8; 256]) -> Option<char> {
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
    (decoded.len() == 1).then_some(decoded[0])
}

fn inject_replacement(action: &ReplacementAction) -> Result<(), RuntimeError> {
    send_inputs(&build_replacement_inputs(action))
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
