use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT_KEYBOARD, KEYEVENTF_KEYUP, VK_BACK, VK_CONTROL, VK_RETURN, VK_TAB,
};

use super::keyboard_runtime::{
    KeyDownDisposition, RuntimeError, build_ctrl_chord_inputs, build_replacement_inputs,
    foreground_change_requires_invalidation, injected_marker, is_shift_modifier_key, is_toggle_key,
    keyup_suppression_after_injection, mouse_message_invalidates_tracking, preclassify_key_down,
};
use crate::correction::{CorrectionDecision, ReplacementText};
use crate::input::{Boundary, CompletedToken, InputEvent};
use crate::replacement::ReplacementEngine;

fn action(
    source: &str,
    replacement: &str,
    boundary: Boundary,
) -> crate::replacement::ReplacementAction {
    ReplacementEngine::new()
        .plan(
            &CompletedToken::new(source, boundary),
            CorrectionDecision::Replace(ReplacementText::try_new(replacement).unwrap()),
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
