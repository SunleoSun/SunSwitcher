use std::mem::{size_of, zeroed};
use std::ptr::null_mut;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, GetKeyboardLayout, GetKeyboardLayoutList, INPUT, INPUT_KEYBOARD,
    KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, SendInput, ToUnicodeEx,
    VK_ADD, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME,
    VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_NEXT, VK_NUMLOCK,
    VK_OEM_1, VK_OEM_3, VK_OEM_4, VK_OEM_6, VK_OEM_7, VK_OEM_COMMA, VK_OEM_PERIOD, VK_PAUSE,
    VK_PRIOR, VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SCROLL, VK_SHIFT,
    VK_SUBTRACT, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId,
    HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, PeekMessageW,
    PostMessageW, PostThreadMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_INPUTLANGCHANGEREQUEST, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_SYSKEYUP,
    WM_USER, WM_XBUTTONDOWN,
};

use crate::completion::{CompletionApplyOutcome, CompletionCommand, CompletionCommandResult};
use crate::input::{Boundary, InputEvent, PhysicalKey, TypedCharacter};
use crate::language::{KeyboardLayoutSwitch, LanguageId};
use crate::persistence::UndoHotkey;
use crate::replacement::{
    ReplacementAction, ReplacementOutcome, SelectedReplacementEngine, SelectedReplacementText,
    SelectedTextDecision, UndoOutcome,
};

use super::selected_text_runtime::{SelectedTextSession, SelectionCaptureIntent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardCommand {
    OpenCurrent,
    OpenPinned,
}

const SUNSWITCHER_INJECTED_MARKER: usize = 0x5355_4E53_5749_5443;
const TO_UNICODE_DO_NOT_CHANGE_STATE: u32 = 0x4;
const DOUBLE_SHIFT_WINDOW: Duration = Duration::from_millis(400);

pub trait InputProcessor: Send + 'static {
    fn process(&mut self, event: InputEvent) -> RuntimeDirective;

    fn replacement_outcome(&mut self, _outcome: ReplacementOutcome) {}

    fn undo(&mut self) -> UndoDirective {
        UndoDirective::Pass
    }

    fn delete_user_word(&mut self, _text: &str) {}

    fn undo_outcome(&mut self, _outcome: UndoOutcome) {}

    fn completion_active(&self) -> bool {
        false
    }

    fn switch_layout_text(&self, _text: &str) -> Option<KeyboardLayoutSwitch> {
        None
    }

    fn tracked_layout_switch_text(&self) -> Option<String> {
        None
    }

    fn tracked_layout_switch_applied(&mut self, _text: &str) {}

    fn completion_command(&mut self, _command: CompletionCommand) -> CompletionCommandResult {
        CompletionCommandResult::Pass
    }

    fn clipboard_command(&mut self, _command: ClipboardCommand, _target_window_id: usize) {}

    fn completion_suffix_outcome(&mut self, _outcome: CompletionApplyOutcome) {}

    fn completion_word_outcome(&mut self, _outcome: CompletionApplyOutcome) {}
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
pub(super) enum PauseHotkeyAction {
    DeleteSelectedUserWord(String),
    UndoPreviousCorrection,
}

pub(super) fn pause_hotkey_action(selected_text: Option<&str>) -> PauseHotkeyAction {
    match selected_text {
        Some(text) => PauseHotkeyAction::DeleteSelectedUserWord(text.to_owned()),
        None => PauseHotkeyAction::UndoPreviousCorrection,
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SuppressedKeyUpRoute {
    DownstreamThenSuppress,
    SunSwitcherOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SuppressedKeyUp {
    vk_code: u32,
    route: SuppressedKeyUpRoute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DoubleShiftEvent {
    None,
    Consume,
    Trigger,
}

#[derive(Debug, Default)]
pub(super) struct DoubleShiftTracker {
    last_plain_release: Option<Instant>,
    current_press_plain: bool,
    second_press_active: bool,
}

impl DoubleShiftTracker {
    pub(super) fn observe(
        &mut self,
        vk_code: u32,
        is_key_down: bool,
        was_key_down: bool,
        now: Instant,
    ) -> DoubleShiftEvent {
        if is_shift_modifier_key(vk_code) {
            if is_key_down {
                if was_key_down {
                    return if self.second_press_active {
                        DoubleShiftEvent::Consume
                    } else {
                        DoubleShiftEvent::None
                    };
                }
                let is_double = self.last_plain_release.take().is_some_and(|previous| {
                    now.saturating_duration_since(previous) <= DOUBLE_SHIFT_WINDOW
                });
                self.second_press_active = is_double;
                self.current_press_plain = !is_double;
                return if is_double {
                    DoubleShiftEvent::Consume
                } else {
                    DoubleShiftEvent::None
                };
            }

            if was_key_down {
                if self.second_press_active {
                    self.second_press_active = false;
                    self.current_press_plain = false;
                    self.last_plain_release = None;
                    return DoubleShiftEvent::Trigger;
                }
                if self.current_press_plain {
                    self.last_plain_release = Some(now);
                }
                self.current_press_plain = false;
            }
            return DoubleShiftEvent::None;
        }

        if is_key_down {
            self.current_press_plain = false;
            self.second_press_active = false;
            self.last_plain_release = None;
        }
        DoubleShiftEvent::None
    }
}

const MOUSE_SELECTION_DRAG_THRESHOLD_PX: i32 = 4;
const MOUSE_DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(500);
const MOUSE_DOUBLE_CLICK_DISTANCE_PX: i32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MouseSelectionEffect {
    None,
    InvalidateOnly,
    UserSelectionIntent,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct MouseSelectionTracker {
    left_button_down_at: Option<(i32, i32)>,
    drag_observed: bool,
    last_left_button_up: Option<(Instant, i32, i32)>,
}

impl MouseSelectionTracker {
    pub(super) fn observe(&mut self, message: u32, x: i32, y: i32) -> MouseSelectionEffect {
        self.observe_at(message, x, y, Instant::now())
    }

    pub(super) fn observe_at(
        &mut self,
        message: u32,
        x: i32,
        y: i32,
        now: Instant,
    ) -> MouseSelectionEffect {
        match message {
            WM_LBUTTONDOWN => {
                self.left_button_down_at = Some((x, y));
                self.drag_observed = false;
                MouseSelectionEffect::InvalidateOnly
            }
            WM_MOUSEMOVE => {
                if let Some((start_x, start_y)) = self.left_button_down_at {
                    let moved_x = (x - start_x).abs() >= MOUSE_SELECTION_DRAG_THRESHOLD_PX;
                    let moved_y = (y - start_y).abs() >= MOUSE_SELECTION_DRAG_THRESHOLD_PX;
                    if moved_x || moved_y {
                        self.drag_observed = true;
                    }
                }
                MouseSelectionEffect::None
            }
            WM_LBUTTONUP => {
                let had_drag = self.left_button_down_at.take().is_some() && self.drag_observed;
                self.drag_observed = false;
                let had_double_click = !had_drag
                    && self
                        .last_left_button_up
                        .is_some_and(|(previous, last_x, last_y)| {
                            now.saturating_duration_since(previous) <= MOUSE_DOUBLE_CLICK_WINDOW
                                && (x - last_x).abs() <= MOUSE_DOUBLE_CLICK_DISTANCE_PX
                                && (y - last_y).abs() <= MOUSE_DOUBLE_CLICK_DISTANCE_PX
                        });
                self.last_left_button_up = if had_drag { None } else { Some((now, x, y)) };
                if had_drag || had_double_click {
                    MouseSelectionEffect::UserSelectionIntent
                } else {
                    MouseSelectionEffect::None
                }
            }
            message if mouse_message_invalidates_tracking(message) => {
                self.left_button_down_at = None;
                self.drag_observed = false;
                self.last_left_button_up = None;
                MouseSelectionEffect::InvalidateOnly
            }
            _ => MouseSelectionEffect::None,
        }
    }
}

struct RuntimeState {
    processor: Box<dyn InputProcessor>,
    keyboard_state: [u8; 256],
    suppressed_keyups: Vec<SuppressedKeyUp>,
    foreground_window_id: usize,
    undo_hotkey: UndoHotkey,
    input_revision: u64,
    double_shift: DoubleShiftTracker,
    selection_capture_intent: SelectionCaptureIntent,
    mouse_selection: MouseSelectionTracker,
}

impl RuntimeState {
    fn new(mut processor: Box<dyn InputProcessor>, undo_hotkey: UndoHotkey) -> Self {
        let _ = processor.process(InputEvent::Invalidate);
        Self {
            processor,
            keyboard_state: initial_keyboard_state(),
            suppressed_keyups: Vec::with_capacity(2),
            foreground_window_id: current_foreground_window_id(),
            undo_hotkey,
            input_revision: 0,
            double_shift: DoubleShiftTracker::default(),
            selection_capture_intent: SelectionCaptureIntent::Unknown,
            mouse_selection: MouseSelectionTracker::default(),
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

    fn completion_hotkey_command(&self, vk_code: u32) -> Option<CompletionCommand> {
        let command = completion_hotkey_command(vk_code, &self.keyboard_state)?;
        if command == CompletionCommand::AcceptNextWord && !self.owns_alt_modifier() {
            return None;
        }
        Some(command)
    }

    fn clipboard_hotkey_command(&self, vk_code: u32) -> Option<ClipboardCommand> {
        clipboard_hotkey_command(vk_code, &self.keyboard_state)
    }

    fn suppress_keyup(&mut self, vk_code: u32, route: SuppressedKeyUpRoute) {
        if let Some(existing) = self
            .suppressed_keyups
            .iter_mut()
            .find(|suppressed| suppressed.vk_code == vk_code)
        {
            existing.route = route;
            return;
        }
        self.suppressed_keyups
            .push(SuppressedKeyUp { vk_code, route });
    }

    fn take_suppressed_keyup(&mut self, vk_code: u32) -> Option<SuppressedKeyUpRoute> {
        let index = self
            .suppressed_keyups
            .iter()
            .position(|suppressed| suppressed.vk_code == vk_code)?;
        Some(self.suppressed_keyups.swap_remove(index).route)
    }

    fn owns_alt_modifier(&self) -> bool {
        self.suppressed_keyups.iter().any(|suppressed| {
            suppressed.route == SuppressedKeyUpRoute::SunSwitcherOnly
                && is_alt_modifier_key(suppressed.vk_code)
        })
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

    fn observe_selection_capture_intent(&mut self, vk_code: u32, is_key_down: bool) {
        if !is_key_down {
            return;
        }
        if let Some(intent) = selection_capture_intent_for_navigation(
            vk_code,
            self.keyboard_state[VK_SHIFT as usize] & 0x80 != 0,
        ) {
            self.selection_capture_intent = intent;
        } else if !is_modifier_key(vk_code) {
            self.selection_capture_intent = SelectionCaptureIntent::Unknown;
        }
    }

    fn clear_selection_capture_intent(&mut self) {
        self.selection_capture_intent = SelectionCaptureIntent::Unknown;
    }
}

static RUNTIME: Mutex<Option<RuntimeState>> = Mutex::new(None);
static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);
static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);
const WM_SUNSWITCHER_STOP: u32 = WM_USER + 0x535;
const WM_SUNSWITCHER_DOUBLE_SHIFT: u32 = WM_USER + 0x536;

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
                if message.message == WM_SUNSWITCHER_DOUBLE_SHIFT {
                    let shift_still_down = GetAsyncKeyState(VK_SHIFT as i32) < 0;
                    eprintln!(
                        "[double-shift] deferred dispatch after hook shift_still_down={shift_still_down}"
                    );
                    if shift_still_down {
                        eprintln!(
                            "[double-shift] deferred dispatch aborted because a physical Shift is still down"
                        );
                        invalidate_runtime_tracking();
                        continue;
                    }
                    switch_selected_or_previous_text();
                    continue;
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
                runtime.observe_selection_capture_intent(event.vkCode, is_key_down);
            }
            runtime.clear_selection_capture_intent();
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
                runtime.clear_selection_capture_intent();
                let _ = runtime.processor.process(InputEvent::Invalidate);
            }
        }

        let was_key_down = (event.vkCode as usize) < runtime.keyboard_state.len()
            && runtime.keyboard_state[event.vkCode as usize] & 0x80 != 0;
        let double_shift = if is_physical_keyboard_event(event.flags) {
            runtime
                .double_shift
                .observe(event.vkCode, is_key_down, was_key_down, Instant::now())
        } else {
            DoubleShiftEvent::None
        };
        runtime.update_key_state(event.vkCode, is_key_down);
        runtime.observe_selection_capture_intent(event.vkCode, is_key_down);
        let ownership_stamp = runtime.ownership_stamp();

        if double_shift == DoubleShiftEvent::Trigger {
            let _ = runtime.take_suppressed_keyup(event.vkCode);
            HookEventState::DoubleShift
        } else if is_key_up {
            HookEventState::KeyUp {
                suppression: runtime.take_suppressed_keyup(event.vkCode),
            }
        } else if double_shift == DoubleShiftEvent::Consume
            || completion_modifier_is_sunswitcher_only(
                runtime.processor.completion_active(),
                event.vkCode,
            )
        {
            runtime.suppress_keyup(event.vkCode, SuppressedKeyUpRoute::SunSwitcherOnly);
            HookEventState::SunSwitcherOnlyKeyDown
        } else if let Some(command) = runtime.clipboard_hotkey_command(event.vkCode) {
            HookEventState::ClipboardHotkey {
                command,
                target_window_id: runtime.foreground_window_id,
            }
        } else if runtime.undo_hotkey_matches(event.vkCode) {
            HookEventState::UndoHotkey { ownership_stamp }
        } else if let Some(command) = runtime.completion_hotkey_command(event.vkCode) {
            HookEventState::CompletionHotkey {
                command,
                ownership_stamp,
            }
        } else {
            if is_key_down && is_state_invalidating_key(event.vkCode) {
                let had_tracked_span = runtime.processor.tracked_layout_switch_text().is_some();
                eprintln!(
                    "[double-shift] keyboard navigation/edit vk=0x{:02X} invalidates tracked_span={had_tracked_span}",
                    event.vkCode
                );
            }
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
        HookEventState::KeyUp { suppression } => {
            if !suppressed_keyup_calls_downstream(suppression) {
                return 1;
            }
            let next_result = unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
            if suppression.is_some() {
                1
            } else {
                next_result
            }
        }
        HookEventState::SunSwitcherOnlyKeyDown => 1,
        HookEventState::DoubleShift => {
            // The gesture fires while the second physical Shift key-up is still inside the
            // low-level hook callback. Never run SendInput/clipboard capture from that callback:
            // queue the editing effect so injected selection/copy events can be processed only
            // after the hook returns and Windows completes the physical modifier transition.
            let thread_id = HOOK_THREAD_ID.load(Ordering::Acquire);
            let shift_down_in_hook = unsafe { GetAsyncKeyState(VK_SHIFT as i32) } < 0;
            let queued = thread_id != 0
                && unsafe { PostThreadMessageW(thread_id, WM_SUNSWITCHER_DOUBLE_SHIFT, 0, 0) != 0 };
            eprintln!(
                "[double-shift] trigger queued={queued} shift_down_in_hook={shift_down_in_hook}"
            );
            if !queued {
                eprintln!("[double-shift] failed to queue deferred gesture effect; invalidating");
                invalidate_runtime_tracking();
            }
            1
        }
        HookEventState::KeyDownPass => {
            let next_result = unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
            if next_result != 0 {
                invalidate_runtime_tracking();
            }
            next_result
        }
        HookEventState::ClipboardHotkey {
            command,
            target_window_id,
        } => {
            if let Ok(mut runtime_slot) = RUNTIME.lock()
                && let Some(runtime) = runtime_slot.as_mut()
            {
                runtime
                    .processor
                    .clipboard_command(command, target_window_id);
                runtime.suppress_keyup(event.vkCode, SuppressedKeyUpRoute::SunSwitcherOnly);
            }
            1
        }
        HookEventState::CompletionHotkey {
            command,
            ownership_stamp,
        } => {
            if !runtime_input_ownership_unchanged(ownership_stamp) {
                invalidate_runtime_tracking();
                return 1;
            }

            // Resolve autocomplete ownership before touching the downstream hook chain. If the
            // popup consumes this key, the physical keydown/keyup belongs to SunSwitcher only and
            // must never reach the foreground application. Only an inactive completion session
            // returns Pass and is then forwarded normally.
            let result = RUNTIME
                .lock()
                .ok()
                .and_then(|mut runtime_slot| {
                    runtime_slot
                        .as_mut()
                        .map(|runtime| runtime.processor.completion_command(command))
                })
                .unwrap_or(CompletionCommandResult::Pass);
            match result {
                CompletionCommandResult::Pass => {
                    debug_assert!(completion_result_passes_through(&result));
                    let next_result = unsafe { CallNextHookEx(null_mut(), code, w_param, l_param) };
                    if next_result != 0 {
                        invalidate_runtime_tracking();
                    }
                    next_result
                }
                CompletionCommandResult::Consumed
                | CompletionCommandResult::DeletePrediction(_) => {
                    if let Ok(mut runtime_slot) = RUNTIME.lock()
                        && let Some(runtime) = runtime_slot.as_mut()
                    {
                        runtime.suppress_keyup(event.vkCode, SuppressedKeyUpRoute::SunSwitcherOnly);
                    }
                    1
                }
                CompletionCommandResult::AcceptSuffix(suffix) => {
                    let injection_result = inject_completion_suffix(&suffix);
                    let ownership_unchanged = runtime_input_ownership_unchanged(ownership_stamp);
                    let outcome = if injection_result.is_ok() && ownership_unchanged {
                        CompletionApplyOutcome::Applied
                    } else {
                        CompletionApplyOutcome::Uncertain
                    };
                    if let Ok(mut runtime_slot) = RUNTIME.lock()
                        && let Some(runtime) = runtime_slot.as_mut()
                    {
                        runtime.processor.completion_suffix_outcome(outcome);
                        runtime.suppress_keyup(event.vkCode, SuppressedKeyUpRoute::SunSwitcherOnly);
                    }
                    invalidate_runtime_tracking();
                    if let Err(error) = injection_result {
                        eprintln!("SunSwitcher completion injection failed: {error:?}");
                    } else if !ownership_unchanged {
                        eprintln!("SunSwitcher completion ownership changed during injection");
                    }
                    1
                }
                CompletionCommandResult::AcceptWord(word) => {
                    let injection_result = inject_completion_word(&word);
                    let ownership_unchanged = runtime_input_ownership_unchanged(ownership_stamp);
                    let outcome = if injection_result.is_ok() && ownership_unchanged {
                        CompletionApplyOutcome::Applied
                    } else {
                        CompletionApplyOutcome::Uncertain
                    };
                    if let Ok(mut runtime_slot) = RUNTIME.lock()
                        && let Some(runtime) = runtime_slot.as_mut()
                    {
                        runtime.processor.completion_word_outcome(outcome);
                        runtime.suppress_keyup(event.vkCode, SuppressedKeyUpRoute::SunSwitcherOnly);
                    }
                    if outcome == CompletionApplyOutcome::Uncertain {
                        invalidate_runtime_tracking();
                    }
                    if let Err(error) = injection_result {
                        eprintln!("SunSwitcher completion word injection failed: {error:?}");
                    }
                    1
                }
            }
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

            let selected_text = SelectedTextSession::capture_existing_selection()
                .map(|session| session.selected_text().as_str().to_owned());
            match pause_hotkey_action(selected_text.as_deref()) {
                PauseHotkeyAction::DeleteSelectedUserWord(text) => {
                    if let Ok(mut runtime_slot) = RUNTIME.lock()
                        && let Some(runtime) = runtime_slot.as_mut()
                    {
                        runtime.processor.delete_user_word(&text);
                    }
                }
                PauseHotkeyAction::UndoPreviousCorrection => {
                    // Resolve Undo only after downstream hooks return. A downstream hook may inject
                    // input re-entrantly or change the foreground window while handling Pause.
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
                        let ownership_unchanged =
                            runtime_input_ownership_unchanged(ownership_stamp);
                        let outcome =
                            undo_outcome_after_injection(&injection_result, ownership_unchanged);
                        notify_runtime_undo_outcome(outcome);
                        if outcome == UndoOutcome::Uncertain {
                            invalidate_runtime_tracking();
                        }
                        if let Err(error) = injection_result {
                            eprintln!("SunSwitcher undo injection failed: {error:?}");
                        }
                    }
                }
            }
            if let Ok(mut runtime_slot) = RUNTIME.lock()
                && let Some(runtime) = runtime_slot.as_mut()
            {
                runtime.suppress_keyup(event.vkCode, SuppressedKeyUpRoute::DownstreamThenSuppress);
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
                // Stay desynchronized with respect to prior text; a later fresh character or
                // unambiguous boundary may establish a new owned span.
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
            if replacement_outcome == ReplacementOutcome::Applied
                && let Some(target_language) = action.target_language()
                && !request_foreground_keyboard_layout(target_language)
            {
                eprintln!(
                    "SunSwitcher could not switch the foreground keyboard layout after automatic correction to {:?}",
                    target_language.as_str()
                );
            }
            if let Some(vk_code) =
                keyup_suppression_after_injection(event.vkCode, &injection_result)
            {
                if let Ok(mut runtime_slot) = RUNTIME.lock()
                    && let Some(runtime) = runtime_slot.as_mut()
                {
                    runtime.suppress_keyup(vk_code, SuppressedKeyUpRoute::DownstreamThenSuppress);
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
    SunSwitcherOnlyKeyDown,
    DoubleShift,
    KeyDownPass,
    KeyDownReplace {
        action: ReplacementAction,
        ownership_stamp: InputOwnershipStamp,
    },
    UndoHotkey {
        ownership_stamp: InputOwnershipStamp,
    },
    ClipboardHotkey {
        command: ClipboardCommand,
        target_window_id: usize,
    },
    CompletionHotkey {
        command: CompletionCommand,
        ownership_stamp: InputOwnershipStamp,
    },
    KeyUp {
        suppression: Option<SuppressedKeyUpRoute>,
    },
}

fn switch_captured_text(session: SelectedTextSession) {
    let source = session.selected_text().clone();
    eprintln!(
        "[double-shift] captured source={:?} chars={}",
        source.as_str(),
        source.as_str().chars().count()
    );
    let switch = RUNTIME.lock().ok().and_then(|runtime_slot| {
        runtime_slot
            .as_ref()
            .and_then(|runtime| runtime.processor.switch_layout_text(source.as_str()))
    });
    let Some(switch) = switch else {
        eprintln!(
            "[double-shift] no layout transform for captured source; restoring capture state"
        );
        if let Err(error) = session.finish_without_replacement() {
            eprintln!("SunSwitcher layout-switch selection cleanup failed: {error:?}");
        }
        return;
    };

    eprintln!(
        "[double-shift] transform source={:?} replacement={:?} target_language={}",
        source.as_str(),
        switch.text(),
        switch.target_language().as_str()
    );
    let replacement = match SelectedReplacementText::try_new(switch.text().to_owned()) {
        Ok(replacement) => replacement,
        Err(_) => {
            eprintln!("[double-shift] transformed text failed SelectedReplacementText validation");
            if let Err(error) = session.finish_without_replacement() {
                eprintln!("SunSwitcher layout-switch selection cleanup failed: {error:?}");
            }
            return;
        }
    };
    let Some(action) =
        SelectedReplacementEngine::new().plan(source, SelectedTextDecision::Replace(replacement))
    else {
        eprintln!("[double-shift] replacement engine produced no action");
        if let Err(error) = session.finish_without_replacement() {
            eprintln!("SunSwitcher layout-switch selection cleanup failed: {error:?}");
        }
        return;
    };

    match session.apply_layout_switch(&action) {
        Ok(()) => {
            eprintln!("[double-shift] replacement input applied");
            if !request_foreground_keyboard_layout(switch.target_language()) {
                eprintln!(
                    "SunSwitcher could not switch the foreground keyboard layout to {:?}",
                    switch.target_language().as_str()
                );
            }
        }
        Err(error) => eprintln!("SunSwitcher layout switch failed: {error:?}"),
    }
}

fn switch_selected_or_previous_text() {
    eprintln!("[double-shift] trigger");
    let selection_capture_intent = RUNTIME
        .lock()
        .ok()
        .and_then(|runtime_slot| {
            runtime_slot
                .as_ref()
                .map(|runtime| runtime.selection_capture_intent)
        })
        .unwrap_or_default();

    let tracked_attempt = RUNTIME.lock().ok().and_then(|runtime_slot| {
        let runtime = runtime_slot.as_ref()?;
        let source = runtime.processor.tracked_layout_switch_text()?;
        let switch = runtime.processor.switch_layout_text(&source);
        Some((source, switch))
    });
    if let Some((source, switch)) = tracked_attempt {
        eprintln!(
            "[double-shift] path=tracked-span source={:?} chars={}",
            source,
            source.chars().count()
        );
        if let Some(switch) = switch {
            match inject_tracked_text_replacement(source.chars().count(), switch.text()) {
                Ok(()) => {
                    eprintln!(
                        "[double-shift] tracked replacement={:?} target_language={}",
                        switch.text(),
                        switch.target_language().as_str()
                    );
                    if !request_foreground_keyboard_layout(switch.target_language()) {
                        eprintln!(
                            "SunSwitcher could not switch the foreground keyboard layout to {:?}",
                            switch.target_language().as_str()
                        );
                    }
                    if let Ok(mut runtime_slot) = RUNTIME.lock()
                        && let Some(runtime) = runtime_slot.as_mut()
                    {
                        runtime
                            .processor
                            .tracked_layout_switch_applied(switch.text());
                    }
                }
                Err(error) => {
                    eprintln!("SunSwitcher tracked layout switch failed: {error:?}");
                    invalidate_runtime_tracking();
                }
            }
            return;
        }

        eprintln!(
            "[double-shift] tracked span has no layout transform; dropping ownership and continuing with capture fallback"
        );
        invalidate_runtime_tracking();
    }

    eprintln!(
        "[double-shift] no transformable tracked span; probing explicit selection intent={selection_capture_intent:?}"
    );
    match SelectedTextSession::capture_existing_selection_for_replacement(selection_capture_intent)
    {
        Ok(Some(session)) => {
            eprintln!("[double-shift] path=explicit-selection");
            switch_captured_text(session);
            invalidate_runtime_tracking();
            return;
        }
        Ok(None) => {
            eprintln!("[double-shift] no copyable explicit selection; probing previous text chunk");
        }
        Err(error) => {
            eprintln!("SunSwitcher selected-text capture failed closed: {error:?}");
            invalidate_runtime_tracking();
            return;
        }
    }

    let session = match SelectedTextSession::capture_previous_word() {
        Ok(session) => session,
        Err(error) => {
            eprintln!("SunSwitcher previous-word capture failed: {error:?}");
            None
        }
    };
    if let Some(session) = session {
        eprintln!("[double-shift] path=previous-caret-chunk");
        switch_captured_text(session);
    } else {
        eprintln!("[double-shift] previous-caret-chunk produced no replaceable text");
    }
    invalidate_runtime_tracking();
}

fn request_foreground_keyboard_layout(language: &LanguageId) -> bool {
    let target_primary = match language.as_str() {
        "en" => 0x09usize,
        "ru" => 0x19usize,
        _ => return false,
    };
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_null() {
        return false;
    }
    let thread_id = unsafe { GetWindowThreadProcessId(foreground, null_mut()) };
    let current_layout = unsafe { GetKeyboardLayout(thread_id) };
    if ((current_layout as usize) & 0xffff) & 0x03ff == target_primary {
        return true;
    }

    let count = unsafe { GetKeyboardLayoutList(0, null_mut()) };
    if count <= 0 {
        return false;
    }
    let mut layouts = vec![null_mut(); count as usize];
    let copied = unsafe { GetKeyboardLayoutList(count, layouts.as_mut_ptr()) };
    if copied <= 0 {
        return false;
    }

    let mut target_layout = None;
    for layout in layouts.into_iter().take(copied as usize) {
        let primary = ((layout as usize) & 0xffff) & 0x03ff;
        if primary == target_primary {
            target_layout = Some(layout);
            break;
        }
    }
    let Some(target_layout) = target_layout else {
        return false;
    };

    unsafe {
        PostMessageW(
            foreground,
            WM_INPUTLANGCHANGEREQUEST,
            0,
            target_layout as LPARAM,
        ) != 0
    }
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
        runtime.clear_selection_capture_intent();
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

pub(super) fn is_physical_keyboard_event(flags: u32) -> bool {
    flags & LLKHF_INJECTED == 0
}

pub(super) fn is_foreign_injected_keyboard_event(flags: u32, extra_info: usize) -> bool {
    flags & LLKHF_INJECTED != 0 && extra_info != SUNSWITCHER_INJECTED_MARKER
}

unsafe extern "system" fn mouse_hook_proc(code: i32, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
    if code >= 0
        && let Ok(mut runtime_slot) = RUNTIME.lock()
        && let Some(runtime) = runtime_slot.as_mut()
    {
        let message = w_param as u32;
        let point = unsafe { (*(l_param as *const MSLLHOOKSTRUCT)).pt };
        match runtime.mouse_selection.observe(message, point.x, point.y) {
            MouseSelectionEffect::InvalidateOnly => {
                let had_tracked_span = runtime.processor.tracked_layout_switch_text().is_some();
                eprintln!(
                    "[double-shift] mouse/navigation click message=0x{message:04X} invalidates tracked_span={had_tracked_span}"
                );
                runtime.note_external_input();
                runtime.clear_selection_capture_intent();
                let _ = runtime.processor.process(InputEvent::Invalidate);
            }
            MouseSelectionEffect::UserSelectionIntent => {
                eprintln!("[double-shift] mouse selection intent message=0x{message:04X}");
                runtime.note_external_input();
                runtime.selection_capture_intent = SelectionCaptureIntent::UserSelection;
            }
            MouseSelectionEffect::None => {}
        }
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
    if is_modifier_key(vk_code) {
        return KeyDownDisposition::Ignore;
    }

    // Command-modified editing keys have application-specific semantics (Ctrl+Backspace,
    // Ctrl+Enter, Ctrl+Tab, Alt+..., Win+...). They must invalidate tracked text before any
    // special-key interpretation so SunSwitcher never reinterprets them as ordinary editing.
    if command_modifier_active {
        return KeyDownDisposition::Event(InputEvent::Invalidate);
    }

    // OEM3 is reserved as SunSwitcher's layout-switch service key. Depending on the active
    // layout Windows translates the same physical key to ` or ё, but neither symbol belongs
    // to the tracked token because the key is used to change input language externally.
    if vk_code == VK_OEM_3 as u32 {
        return KeyDownDisposition::Ignore;
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

fn is_alt_modifier_key(vk_code: u32) -> bool {
    [VK_MENU, VK_LMENU, VK_RMENU]
        .into_iter()
        .any(|key| vk_code == key as u32)
}

pub(super) fn completion_modifier_is_sunswitcher_only(
    completion_active: bool,
    vk_code: u32,
) -> bool {
    completion_active && is_alt_modifier_key(vk_code)
}

fn is_modifier_key(vk_code: u32) -> bool {
    is_shift_modifier_key(vk_code)
        || [VK_CONTROL, VK_LCONTROL, VK_RCONTROL, VK_LWIN, VK_RWIN]
            .into_iter()
            .any(|key| vk_code == key as u32)
        || is_alt_modifier_key(vk_code)
}

pub(super) fn completion_result_passes_through(result: &CompletionCommandResult) -> bool {
    matches!(result, CompletionCommandResult::Pass)
}

pub(super) const fn suppressed_keyup_calls_downstream(
    suppression: Option<SuppressedKeyUpRoute>,
) -> bool {
    !matches!(suppression, Some(SuppressedKeyUpRoute::SunSwitcherOnly))
}

pub(super) fn clipboard_hotkey_command(
    vk_code: u32,
    keyboard_state: &[u8; 256],
) -> Option<ClipboardCommand> {
    let alt = keyboard_state[VK_MENU as usize] & 0x80 != 0;
    let ctrl = keyboard_state[VK_CONTROL as usize] & 0x80 != 0;
    let shift = keyboard_state[VK_SHIFT as usize] & 0x80 != 0;
    let win = keyboard_state[VK_LWIN as usize] & 0x80 != 0
        || keyboard_state[VK_RWIN as usize] & 0x80 != 0;
    if ctrl && shift && !alt && !win && vk_code == VK_SUBTRACT as u32 {
        return Some(ClipboardCommand::OpenCurrent);
    }
    if ctrl && shift && !alt && !win && vk_code == VK_ADD as u32 {
        return Some(ClipboardCommand::OpenPinned);
    }
    None
}

pub(super) fn completion_hotkey_command(
    vk_code: u32,
    keyboard_state: &[u8; 256],
) -> Option<CompletionCommand> {
    let alt = keyboard_state[VK_MENU as usize] & 0x80 != 0;
    let ctrl = keyboard_state[VK_CONTROL as usize] & 0x80 != 0;
    let shift = keyboard_state[VK_SHIFT as usize] & 0x80 != 0;
    let win = keyboard_state[VK_LWIN as usize] & 0x80 != 0
        || keyboard_state[VK_RWIN as usize] & 0x80 != 0;

    if alt && !ctrl && !shift && !win && vk_code == VK_RIGHT as u32 {
        return Some(CompletionCommand::AcceptNextWord);
    }
    if alt || ctrl || shift || win {
        return None;
    }

    match vk_code as u16 {
        VK_ESCAPE => Some(CompletionCommand::Dismiss),
        VK_UP => Some(CompletionCommand::Previous),
        VK_DOWN => Some(CompletionCommand::Next),
        VK_TAB => Some(CompletionCommand::Accept),
        VK_DELETE => Some(CompletionCommand::DeleteSelected),
        _ => None,
    }
}

fn is_selection_navigation_key(vk_code: u32) -> bool {
    [
        VK_HOME, VK_END, VK_PRIOR, VK_NEXT, VK_LEFT, VK_RIGHT, VK_UP, VK_DOWN,
    ]
    .into_iter()
    .any(|key| vk_code == key as u32)
}

pub(super) fn selection_capture_intent_for_navigation(
    vk_code: u32,
    shift_active: bool,
) -> Option<SelectionCaptureIntent> {
    is_selection_navigation_key(vk_code).then_some(if shift_active {
        SelectionCaptureIntent::UserSelection
    } else {
        SelectionCaptureIntent::Unknown
    })
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

pub(super) fn inject_selected_replacement(text: &str) -> Result<(), RuntimeError> {
    send_inputs(&build_selected_replacement_inputs(text))
}

pub(super) fn inject_backward_selection(select_left_chars: usize) -> Result<(), RuntimeError> {
    send_inputs(&build_backward_selection_inputs(select_left_chars))
}

pub(super) fn inject_current_caret_forward_selection(
    select_right_chars: usize,
) -> Result<(), RuntimeError> {
    send_inputs(&build_current_caret_forward_selection_inputs(
        select_right_chars,
    ))
}

pub(super) fn inject_current_caret_forward_replacement(
    select_right_chars: usize,
    text: &str,
) -> Result<(), RuntimeError> {
    send_inputs(&build_current_caret_forward_replacement_inputs(
        select_right_chars,
        text,
    ))
}

pub(super) fn inject_previous_caret_range_replacement(
    selected_chars: usize,
    trailing_chars: usize,
    text: &str,
) -> Result<(), RuntimeError> {
    send_inputs(&build_previous_caret_range_replacement_inputs(
        selected_chars,
        trailing_chars,
        text,
    ))
}

pub(super) fn inject_key_presses(key: u16, count: usize) -> Result<(), RuntimeError> {
    send_inputs(&build_key_presses_inputs(key, count))
}

fn inject_tracked_text_replacement(
    delete_previous_chars: usize,
    text: &str,
) -> Result<(), RuntimeError> {
    send_inputs(&build_tracked_text_replacement_inputs(
        delete_previous_chars,
        text,
    ))
}

fn inject_completion_suffix(text: &str) -> Result<(), RuntimeError> {
    send_inputs(&build_completion_suffix_inputs(text))
}

fn inject_completion_word(text: &str) -> Result<(), RuntimeError> {
    send_inputs(&build_completion_word_inputs(text))
}

pub(super) fn build_completion_suffix_inputs(text: &str) -> Vec<INPUT> {
    let mut inputs = Vec::with_capacity(text.encode_utf16().count() * 2);
    for unit in text.encode_utf16() {
        inputs.push(unicode_input(unit, false));
        inputs.push(unicode_input(unit, true));
    }
    inputs
}

pub(super) fn build_completion_word_inputs(text: &str) -> Vec<INPUT> {
    build_completion_suffix_inputs(text)
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

pub(super) fn build_selected_replacement_inputs(text: &str) -> Vec<INPUT> {
    let mut inputs = build_selected_text_inputs("");
    if !text.is_empty() {
        inputs.extend(build_selected_text_inputs(text));
    }
    inputs
}

pub(super) fn build_backward_selection_inputs(select_left_chars: usize) -> Vec<INPUT> {
    let mut inputs = Vec::with_capacity(select_left_chars.saturating_mul(2) + 2);
    inputs.push(virtual_key_input(VK_LSHIFT, false));
    append_key_presses(&mut inputs, VK_LEFT, select_left_chars);
    inputs.push(virtual_key_input(VK_LSHIFT, true));
    inputs
}

fn append_key_presses(inputs: &mut Vec<INPUT>, key: u16, count: usize) {
    for _ in 0..count {
        inputs.push(virtual_key_input(key, false));
        inputs.push(virtual_key_input(key, true));
    }
}

pub(super) fn build_current_caret_forward_selection_inputs(
    select_right_chars: usize,
) -> Vec<INPUT> {
    let mut inputs = Vec::with_capacity(select_right_chars.saturating_mul(2) + 2);
    inputs.push(virtual_key_input(VK_LSHIFT, false));
    append_key_presses(&mut inputs, VK_RIGHT, select_right_chars);
    inputs.push(virtual_key_input(VK_LSHIFT, true));
    inputs
}

pub(super) fn build_current_caret_forward_replacement_inputs(
    select_right_chars: usize,
    text: &str,
) -> Vec<INPUT> {
    let mut inputs = build_current_caret_forward_selection_inputs(select_right_chars);
    inputs.extend(build_selected_replacement_inputs(text));
    inputs
}

pub(super) fn build_key_presses_inputs(key: u16, count: usize) -> Vec<INPUT> {
    let mut inputs = Vec::with_capacity(count.saturating_mul(2));
    append_key_presses(&mut inputs, key, count);
    inputs
}

pub(super) fn build_previous_caret_range_replacement_inputs(
    selected_chars: usize,
    trailing_chars: usize,
    text: &str,
) -> Vec<INPUT> {
    let mut inputs = build_key_presses_inputs(VK_LEFT, trailing_chars);
    inputs.extend(build_backward_selection_inputs(selected_chars));
    inputs.extend(build_selected_replacement_inputs(text));
    append_key_presses(&mut inputs, VK_RIGHT, trailing_chars);
    inputs
}

pub(super) fn build_tracked_text_replacement_inputs(
    delete_previous_chars: usize,
    text: &str,
) -> Vec<INPUT> {
    let mut inputs = Vec::with_capacity(
        delete_previous_chars.saturating_mul(2) + text.encode_utf16().count().saturating_mul(2),
    );
    for _ in 0..delete_previous_chars {
        inputs.push(virtual_key_input(VK_BACK, false));
        inputs.push(virtual_key_input(VK_BACK, true));
    }
    inputs.extend(build_completion_suffix_inputs(text));
    inputs
}

pub(super) fn inject_ctrl_chord(key: u16) -> Result<(), RuntimeError> {
    send_inputs(&build_ctrl_chord_inputs(key))
}

pub(super) fn inject_ctrl_shift_chord(key: u16) -> Result<(), RuntimeError> {
    send_inputs(&build_ctrl_shift_chord_inputs(key))
}

pub(super) fn inject_key_press(key: u16) -> Result<(), RuntimeError> {
    send_inputs(&[virtual_key_input(key, false), virtual_key_input(key, true)])
}

// Selected-text reconstruction now uses explicit line-relative and forward-selection builders.

pub(super) fn build_ctrl_chord_inputs(key: u16) -> Vec<INPUT> {
    vec![
        virtual_key_input(VK_LCONTROL, false),
        virtual_key_input(key, false),
        virtual_key_input(key, true),
        virtual_key_input(VK_LCONTROL, true),
    ]
}

pub(super) fn build_ctrl_shift_chord_inputs(key: u16) -> Vec<INPUT> {
    vec![
        virtual_key_input(VK_LCONTROL, false),
        virtual_key_input(VK_LSHIFT, false),
        virtual_key_input(key, false),
        virtual_key_input(key, true),
        virtual_key_input(VK_LSHIFT, true),
        virtual_key_input(VK_LCONTROL, true),
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
    let mut inputs = build_tracked_text_replacement_inputs(
        action.delete_previous_chars(),
        action.replacement().as_str(),
    );
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
    let is_extended_navigation = matches!(
        key,
        VK_INSERT
            | VK_DELETE
            | VK_HOME
            | VK_END
            | VK_PRIOR
            | VK_NEXT
            | VK_LEFT
            | VK_RIGHT
            | VK_UP
            | VK_DOWN
    );
    input.Anonymous.ki = KEYBDINPUT {
        wVk: key,
        wScan: 0,
        dwFlags: (if is_extended_navigation {
            KEYEVENTF_EXTENDEDKEY
        } else {
            0
        }) | if key_up { KEYEVENTF_KEYUP } else { 0 },
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
