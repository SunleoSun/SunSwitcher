use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT_KEYBOARD, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN,
    VK_ESCAPE, VK_LEFT, VK_MENU, VK_OEM_1, VK_OEM_3, VK_OEM_4, VK_OEM_6, VK_OEM_7, VK_OEM_COMMA,
    VK_OEM_PERIOD, VK_PAUSE, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP,
};

use windows_sys::Win32::UI::WindowsAndMessaging::LLKHF_INJECTED;

use super::keyboard_runtime::{
    DoubleShiftEvent, DoubleShiftTracker, InputOwnershipStamp, KeyDownDisposition,
    PauseHotkeyAction, RuntimeError, build_completion_suffix_inputs, build_completion_word_inputs,
    build_ctrl_chord_inputs, build_ctrl_shift_chord_inputs, build_previous_word_selection_inputs,
    build_replacement_inputs, completion_hotkey_command, foreground_change_requires_invalidation,
    injected_marker, input_ownership_matches, is_foreign_injected_keyboard_event,
    is_physical_keyboard_event, is_shift_modifier_key, is_toggle_key,
    keyup_suppression_after_injection, mouse_message_invalidates_tracking, pause_hotkey_action,
    physical_key_from_vk, preclassify_key_down, undo_hotkey_matches, undo_outcome_after_injection,
    update_keyboard_state,
};
use crate::completion::CompletionCommand;
use crate::correction::{CorrectionDecision, ReplacementText};
use crate::input::{Boundary, CompletedToken, InputEvent, PhysicalKey};
use crate::persistence::UndoHotkey;
use crate::replacement::{ReplacementEngine, UndoOutcome};

fn action(
    source: &str,
    replacement: &str,
    boundary: Boundary,
) -> crate::replacement::ReplacementAction {
    ReplacementEngine::new()
        .plan(
            &CompletedToken::new(source, boundary),
            CorrectionDecision::Replace(ReplacementText::try_new(replacement).unwrap().into()),
        )
        .unwrap()
}

#[test]
fn mouse_clicks_invalidate_tracked_text_state() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_XBUTTONDOWN,
    };

    assert!(mouse_message_invalidates_tracking(WM_LBUTTONDOWN));
    assert!(mouse_message_invalidates_tracking(WM_RBUTTONDOWN));
    assert!(mouse_message_invalidates_tracking(WM_MBUTTONDOWN));
    assert!(mouse_message_invalidates_tracking(WM_XBUTTONDOWN));
    assert!(!mouse_message_invalidates_tracking(WM_MOUSEMOVE));
}

#[test]
fn foreign_injected_keyboard_events_fail_closed_but_owned_injection_does_not() {
    assert!(is_foreign_injected_keyboard_event(LLKHF_INJECTED, 0));
    assert!(!is_foreign_injected_keyboard_event(
        LLKHF_INJECTED,
        injected_marker()
    ));
    assert!(!is_foreign_injected_keyboard_event(0, 0));

    let mut state = [0u8; 256];
    update_keyboard_state(&mut state, VK_CONTROL as u32, true);
    assert_ne!(state[VK_CONTROL as usize] & 0x80, 0);

    update_keyboard_state(&mut state, b'A' as u32, true);
    assert_ne!(
        state[VK_CONTROL as usize] & 0x80,
        0,
        "a generic injected modifier must survive unrelated key transitions"
    );

    update_keyboard_state(&mut state, VK_CONTROL as u32, false);
    assert_eq!(state[VK_CONTROL as usize] & 0x80, 0);
}

#[test]
fn pause_undo_hotkey_requires_an_unmodified_keypress() {
    let mut state = [0u8; 256];
    assert!(undo_hotkey_matches(
        UndoHotkey::Pause,
        VK_PAUSE as u32,
        &state
    ));
    assert!(!undo_hotkey_matches(UndoHotkey::Pause, b'A' as u32, &state));

    state[VK_SHIFT as usize] = 0x80;
    assert!(!undo_hotkey_matches(
        UndoHotkey::Pause,
        VK_PAUSE as u32,
        &state
    ));
}

#[test]
fn grave_oem_key_is_service_input_instead_of_token_text() {
    assert_eq!(
        preclassify_key_down(VK_OEM_3 as u32, false),
        KeyDownDisposition::Ignore
    );
}

#[test]
fn pause_with_selection_deletes_that_user_word_instead_of_undoing() {
    assert_eq!(
        pause_hotkey_action(Some("CustomToken")),
        PauseHotkeyAction::DeleteSelectedUserWord("CustomToken".to_owned())
    );
    assert_eq!(
        pause_hotkey_action(None),
        PauseHotkeyAction::UndoPreviousCorrection
    );
}

#[test]
fn completion_hotkeys_require_exact_default_modifier_contract() {
    let mut state = [0u8; 256];
    assert_eq!(
        completion_hotkey_command(VK_ESCAPE as u32, &state),
        Some(CompletionCommand::Dismiss)
    );
    assert_eq!(
        completion_hotkey_command(VK_UP as u32, &state),
        Some(CompletionCommand::Previous)
    );
    assert_eq!(
        completion_hotkey_command(VK_DOWN as u32, &state),
        Some(CompletionCommand::Next)
    );
    assert_eq!(
        completion_hotkey_command(VK_RETURN as u32, &state),
        Some(CompletionCommand::Accept)
    );
    assert_eq!(
        completion_hotkey_command(VK_TAB as u32, &state),
        Some(CompletionCommand::Accept)
    );
    assert_eq!(
        completion_hotkey_command(VK_DELETE as u32, &state),
        Some(CompletionCommand::DeleteSelected)
    );

    update_keyboard_state(&mut state, VK_MENU as u32, true);
    assert_eq!(
        completion_hotkey_command(VK_RIGHT as u32, &state),
        Some(CompletionCommand::AcceptNextWord)
    );
    assert_eq!(completion_hotkey_command(VK_UP as u32, &state), None);
    update_keyboard_state(&mut state, VK_MENU as u32, false);
    update_keyboard_state(&mut state, VK_CONTROL as u32, true);
    assert_eq!(completion_hotkey_command(VK_RETURN as u32, &state), None);
}

#[test]
fn double_shift_ignores_injected_keyboard_transitions() {
    assert!(is_physical_keyboard_event(0));
    assert!(!is_physical_keyboard_event(LLKHF_INJECTED));
}

#[test]
fn double_shift_triggers_only_after_two_plain_taps_and_on_second_release() {
    use std::time::{Duration, Instant};

    let mut tracker = DoubleShiftTracker::default();
    let start = Instant::now();
    assert_eq!(
        tracker.observe(VK_SHIFT as u32, true, false, start),
        DoubleShiftEvent::None
    );
    assert_eq!(
        tracker.observe(
            VK_SHIFT as u32,
            false,
            true,
            start + Duration::from_millis(40)
        ),
        DoubleShiftEvent::None
    );
    assert_eq!(
        tracker.observe(
            VK_SHIFT as u32,
            true,
            false,
            start + Duration::from_millis(100)
        ),
        DoubleShiftEvent::Consume
    );
    assert_eq!(
        tracker.observe(
            VK_SHIFT as u32,
            false,
            true,
            start + Duration::from_millis(130)
        ),
        DoubleShiftEvent::Trigger
    );

    assert_eq!(
        tracker.observe(
            VK_SHIFT as u32,
            true,
            false,
            start + Duration::from_millis(500)
        ),
        DoubleShiftEvent::None
    );
    assert_eq!(
        tracker.observe(
            VK_SHIFT as u32,
            false,
            true,
            start + Duration::from_millis(520)
        ),
        DoubleShiftEvent::None
    );
    assert_eq!(
        tracker.observe(b'A' as u32, true, false, start + Duration::from_millis(530)),
        DoubleShiftEvent::None
    );
    assert_eq!(
        tracker.observe(
            VK_SHIFT as u32,
            true,
            false,
            start + Duration::from_millis(550)
        ),
        DoubleShiftEvent::None
    );
}

#[test]
fn ctrl_shift_selection_chord_has_balanced_modifier_order() {
    let inputs = build_ctrl_shift_chord_inputs(VK_RIGHT);
    assert_eq!(inputs.len(), 6);
    let keys = inputs
        .iter()
        .map(|input| unsafe { input.Anonymous.ki })
        .collect::<Vec<_>>();
    assert_eq!(keys[0].wVk, VK_CONTROL);
    assert_eq!(keys[1].wVk, VK_SHIFT);
    assert_eq!(keys[2].wVk, VK_RIGHT);
    assert_eq!(keys[3].wVk, VK_RIGHT);
    assert_eq!(keys[4].wVk, VK_SHIFT);
    assert_eq!(keys[5].wVk, VK_CONTROL);
    assert_eq!(keys[0].dwFlags & KEYEVENTF_KEYUP, 0);
    assert_eq!(keys[1].dwFlags & KEYEVENTF_KEYUP, 0);
    assert_ne!(keys[3].dwFlags & KEYEVENTF_KEYUP, 0);
    assert_ne!(keys[4].dwFlags & KEYEVENTF_KEYUP, 0);
    assert_ne!(keys[5].dwFlags & KEYEVENTF_KEYUP, 0);
}

#[test]
fn previous_word_reselection_normalizes_to_the_captured_word_before_delete() {
    let inputs = build_previous_word_selection_inputs();
    let keys = inputs
        .iter()
        .map(|input| unsafe { input.Anonymous.ki })
        .collect::<Vec<_>>();

    assert_eq!(keys.len(), 12);
    assert_eq!(keys[0].wVk, VK_LEFT);
    assert_eq!(keys[1].wVk, VK_LEFT);
    assert_eq!(keys[2].wVk, VK_CONTROL);
    assert_eq!(keys[3].wVk, VK_RIGHT);
    assert_eq!(keys[6].wVk, VK_CONTROL);
    assert_eq!(keys[7].wVk, VK_SHIFT);
    assert_eq!(keys[8].wVk, VK_LEFT);
}

#[test]
fn completion_suffix_uses_only_unicode_inputs() {
    let inputs = build_completion_suffix_inputs("лать");
    assert_eq!(inputs.len(), "лать".encode_utf16().count() * 2);
    assert!(inputs.iter().all(|input| {
        let key = unsafe { input.Anonymous.ki };
        key.wVk == 0 && key.dwFlags & KEYEVENTF_UNICODE != 0
    }));
}

#[test]
fn completion_word_uses_only_unicode_while_physical_alt_is_owned_by_sunswitcher() {
    let inputs = build_completion_word_inputs("лать ");
    assert_eq!(inputs.len(), "лать ".encode_utf16().count() * 2);
    assert!(inputs.iter().all(|input| {
        let key = unsafe { input.Anonymous.ki };
        key.wVk == 0 && key.dwFlags & KEYEVENTF_UNICODE != 0
    }));
}

#[test]
fn in_flight_ownership_requires_same_revision_and_foreground_window() {
    let stamp = InputOwnershipStamp::new(7, 42);
    assert!(input_ownership_matches(stamp, 7, 42, 42));
    assert!(!input_ownership_matches(stamp, 8, 42, 42));
    assert!(!input_ownership_matches(stamp, 7, 42, 99));
    assert!(!input_ownership_matches(stamp, 7, 99, 42));
}

#[test]
fn foreground_window_change_requires_invalidation() {
    assert!(!foreground_change_requires_invalidation(100, 100));
    assert!(foreground_change_requires_invalidation(100, 200));
}

#[test]
fn command_modified_editing_keys_preempt_normal_special_key_semantics() {
    for vk_code in [VK_BACK, VK_RETURN, VK_TAB] {
        assert_eq!(
            preclassify_key_down(vk_code as u32, true),
            KeyDownDisposition::Event(InputEvent::Invalidate)
        );
    }

    assert_eq!(
        preclassify_key_down(VK_BACK as u32, false),
        KeyDownDisposition::Event(InputEvent::Backspace)
    );
    assert_eq!(
        preclassify_key_down(VK_RETURN as u32, false),
        KeyDownDisposition::Event(InputEvent::Boundary(Boundary::Enter))
    );
    assert_eq!(
        preclassify_key_down(VK_TAB as u32, false),
        KeyDownDisposition::Event(InputEvent::Boundary(Boundary::Tab))
    );
}

#[test]
fn undo_injection_distinguishes_no_effect_from_uncertain_partial_effect() {
    let not_executed = Err(RuntimeError::InjectionFailed {
        expected: 8,
        sent: 0,
    });
    let uncertain = Err(RuntimeError::InjectionFailed {
        expected: 8,
        sent: 2,
    });
    let applied: Result<(), RuntimeError> = Ok(());

    assert_eq!(
        undo_outcome_after_injection(&not_executed, true),
        UndoOutcome::NotExecuted
    );
    assert_eq!(
        undo_outcome_after_injection(&uncertain, true),
        UndoOutcome::Uncertain
    );
    assert_eq!(
        undo_outcome_after_injection(&applied, true),
        UndoOutcome::Applied
    );
    assert_eq!(
        undo_outcome_after_injection(&applied, false),
        UndoOutcome::Uncertain
    );
    assert_eq!(
        undo_outcome_after_injection(&not_executed, false),
        UndoOutcome::Uncertain
    );
}

#[test]
fn failed_replacement_injection_does_not_suppress_physical_keyup() {
    let failed = Err(RuntimeError::InjectionFailed {
        expected: 8,
        sent: 0,
    });
    let succeeded: Result<(), RuntimeError> = Ok(());

    assert_eq!(
        keyup_suppression_after_injection(VK_TAB as u32, &failed),
        None
    );
    assert_eq!(
        keyup_suppression_after_injection(VK_TAB as u32, &succeeded),
        Some(VK_TAB as u32)
    );
}

#[test]
fn windows_oem_keys_map_to_layout_ambiguous_physical_identity() {
    for (vk_code, expected) in [
        (VK_OEM_3, PhysicalKey::Grave),
        (VK_OEM_4, PhysicalKey::LeftBracket),
        (VK_OEM_6, PhysicalKey::RightBracket),
        (VK_OEM_1, PhysicalKey::Semicolon),
        (VK_OEM_7, PhysicalKey::Quote),
        (VK_OEM_COMMA, PhysicalKey::Comma),
        (VK_OEM_PERIOD, PhysicalKey::Period),
    ] {
        assert_eq!(physical_key_from_vk(vk_code as u32), expected);
    }
}

#[test]
fn toggle_keys_are_explicit_keyboard_state() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_CAPITAL, VK_NUMLOCK, VK_SCROLL};

    assert!(is_toggle_key(VK_CAPITAL as u32));
    assert!(is_toggle_key(VK_NUMLOCK as u32));
    assert!(is_toggle_key(VK_SCROLL as u32));
}

#[test]
fn shift_modifier_is_transparent_to_token_tracking() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_LSHIFT, VK_RSHIFT, VK_SHIFT};

    assert!(is_shift_modifier_key(VK_SHIFT as u32));
    assert!(is_shift_modifier_key(VK_LSHIFT as u32));
    assert!(is_shift_modifier_key(VK_RSHIFT as u32));
}

#[test]
fn builds_backspace_text_and_boundary_input_sequence() {
    let inputs = build_replacement_inputs(&action("abcx", "OK", Boundary::Enter));
    assert_eq!(inputs.len(), 4 * 2 + 2 * 2 + 2);
    assert!(inputs.iter().all(|input| input.r#type == INPUT_KEYBOARD));
}

#[test]
fn ctrl_chord_has_explicit_press_and_release_order() {
    let inputs = build_ctrl_chord_inputs(b'C' as u16);
    assert_eq!(inputs.len(), 4);
    let keys: Vec<_> = inputs
        .iter()
        .map(|input| unsafe { input.Anonymous.ki })
        .collect();
    assert_eq!(keys[0].wVk, VK_CONTROL);
    assert_eq!(keys[1].wVk, b'C' as u16);
    assert_eq!(keys[2].wVk, b'C' as u16);
    assert_eq!(keys[3].wVk, VK_CONTROL);
    assert_eq!(keys[0].dwFlags & KEYEVENTF_KEYUP, 0);
    assert_eq!(keys[1].dwFlags & KEYEVENTF_KEYUP, 0);
    assert_ne!(keys[2].dwFlags & KEYEVENTF_KEYUP, 0);
    assert_ne!(keys[3].dwFlags & KEYEVENTF_KEYUP, 0);
}

#[test]
fn every_generated_keyboard_event_is_owned_by_sunswitcher() {
    let mut inputs = build_replacement_inputs(&action("дял", "для", Boundary::Character(' ')));
    inputs.extend(build_ctrl_chord_inputs(b'V' as u16));
    for input in inputs {
        let keyboard = unsafe { input.Anonymous.ki };
        assert_eq!(keyboard.dwExtraInfo, injected_marker());
    }
}

#[test]
fn generated_events_are_balanced_down_up_pairs() {
    let inputs = build_replacement_inputs(&action("дял", "для", Boundary::Character('.')));
    assert_eq!(inputs.len() % 2, 0);
    for pair in inputs.chunks_exact(2) {
        let down = unsafe { pair[0].Anonymous.ki };
        let up = unsafe { pair[1].Anonymous.ki };
        assert_eq!(down.dwFlags & KEYEVENTF_KEYUP, 0);
        assert_ne!(up.dwFlags & KEYEVENTF_KEYUP, 0);
    }
}
