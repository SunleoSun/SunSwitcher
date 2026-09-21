use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VK_BACK, VK_OEM_1, VK_OEM_3, VK_OEM_4, VK_OEM_6, VK_OEM_7,
    VK_OEM_COMMA, VK_OEM_PERIOD, VK_PAUSE, VK_RETURN, VK_TAB,
};

use windows_sys::Win32::UI::WindowsAndMessaging::LLKHF_INJECTED;

use super::keyboard_runtime::{
    InputOwnershipStamp, KeyDownDisposition, RuntimeError, build_replacement_inputs,
    foreground_change_requires_invalidation, injected_marker, input_ownership_matches,
    is_foreign_injected_keyboard_event, keyup_suppression_after_injection, physical_key_from_vk,
    preclassify_key_down, replacement_outcome_after_injection, undo_hotkey_matches,
    undo_outcome_after_injection,
};
use crate::correction::{CorrectionDecision, ReplacementText};
use crate::input::{Boundary, CompletedToken, InputBuffer, InputEvent, InputOutcome, PhysicalKey};
use crate::persistence::UndoHotkey;
use crate::replacement::{ReplacementEngine, ReplacementOutcome, UndoOutcome};

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
fn certification_replacement_sequence_preserves_delete_then_insert_then_boundary_order() {
    let inputs = build_replacement_inputs(&action("дял", "для", Boundary::Character('!')));

    for pair in inputs[..6].chunks_exact(2) {
        let down = unsafe { pair[0].Anonymous.ki };
        assert_eq!(down.wVk, VK_BACK);
    }

    let replacement_units: Vec<u16> = "для".encode_utf16().collect();
    for (pair, expected_unit) in inputs[6..12].chunks_exact(2).zip(replacement_units) {
        let down = unsafe { pair[0].Anonymous.ki };
        assert_eq!(down.wScan, expected_unit);
        assert_ne!(down.dwFlags & KEYEVENTF_UNICODE, 0);
    }

    let boundary = unsafe { inputs[12].Anonymous.ki };
    assert_eq!(boundary.wScan, '!' as u16);
    assert_ne!(boundary.dwFlags & KEYEVENTF_UNICODE, 0);
}

#[test]
fn certification_enter_boundary_is_replayed_as_virtual_key() {
    let inputs = build_replacement_inputs(&action("abcx", "OK", Boundary::Enter));
    let boundary_down = unsafe { inputs[inputs.len() - 2].Anonymous.ki };
    assert_eq!(boundary_down.wVk, VK_RETURN);
    assert_eq!(boundary_down.dwFlags & KEYEVENTF_KEYUP, 0);
}

#[test]
fn certification_tab_boundary_normalizes_held_key_then_replays_a_full_press() {
    let inputs = build_replacement_inputs(&action("abcx", "OK", Boundary::Tab));
    let replay = &inputs[inputs.len() - 3..];
    let release_held = unsafe { replay[0].Anonymous.ki };
    let press = unsafe { replay[1].Anonymous.ki };
    let release = unsafe { replay[2].Anonymous.ki };

    assert_eq!(release_held.wVk, VK_TAB);
    assert_ne!(release_held.dwFlags & KEYEVENTF_KEYUP, 0);
    assert_eq!(press.wVk, VK_TAB);
    assert_eq!(press.dwFlags & KEYEVENTF_KEYUP, 0);
    assert_eq!(release.wVk, VK_TAB);
    assert_ne!(release.dwFlags & KEYEVENTF_KEYUP, 0);
}

#[test]
fn certification_cyrillic_replacement_is_encoded_as_utf16_unicode_events() {
    let inputs = build_replacement_inputs(&action("дял", "для", Boundary::Character(' ')));
    let replacement_start = 3 * 2;
    let replacement_end = replacement_start + "для".encode_utf16().count() * 2;
    let units: Vec<u16> = inputs[replacement_start..replacement_end]
        .chunks_exact(2)
        .map(|pair| unsafe { pair[0].Anonymous.ki.wScan })
        .collect();

    assert_eq!(units, "для".encode_utf16().collect::<Vec<_>>());
}

#[test]
fn certification_windows_runtime_preserves_layout_ambiguous_physical_keys() {
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
fn certification_pause_is_the_default_unmodified_undo_binding() {
    let state = [0u8; 256];
    assert_eq!(UndoHotkey::DEFAULT, UndoHotkey::Pause);
    assert!(undo_hotkey_matches(
        UndoHotkey::DEFAULT,
        VK_PAUSE as u32,
        &state
    ));
}

#[test]
fn certification_foreign_injected_input_cannot_become_owned_text_state() {
    assert!(is_foreign_injected_keyboard_event(LLKHF_INJECTED, 0));
    assert!(!is_foreign_injected_keyboard_event(
        LLKHF_INJECTED,
        injected_marker()
    ));
}

#[test]
fn certification_command_modified_editing_keys_fail_closed() {
    for vk_code in [VK_BACK, VK_RETURN, VK_TAB] {
        assert_eq!(
            preclassify_key_down(vk_code as u32, true),
            KeyDownDisposition::Event(InputEvent::Invalidate)
        );
    }
}

#[test]
fn certification_reentrant_foreground_change_invalidates_inflight_side_effect_ownership() {
    let stamp = InputOwnershipStamp::new(11, 100);
    assert!(input_ownership_matches(stamp, 11, 100, 100));
    assert!(!input_ownership_matches(stamp, 11, 100, 200));
    assert!(!input_ownership_matches(stamp, 12, 100, 100));
}

#[test]
fn certification_foreground_change_discards_stale_token_but_fresh_typing_restarts_tracking() {
    let mut buffer = InputBuffer::new();
    for character in "дял".chars() {
        assert_eq!(
            buffer.process(InputEvent::character(character)),
            InputOutcome::Continue
        );
    }

    assert!(foreground_change_requires_invalidation(100, 200));
    assert_eq!(
        buffer.process(InputEvent::Invalidate),
        InputOutcome::Invalidated
    );
    for character in "привте".chars() {
        assert_eq!(
            buffer.process(InputEvent::character(character)),
            InputOutcome::Continue
        );
    }
    let InputOutcome::Completed(token) = buffer.process(InputEvent::character(' ')) else {
        panic!("fresh typing after foreground invalidation should be tracked immediately");
    };
    assert_eq!(token.text(), "привте");
}

#[test]
fn certification_undo_retry_is_allowed_only_when_injection_sent_nothing() {
    let not_executed = Err(RuntimeError::InjectionFailed {
        expected: 12,
        sent: 0,
    });
    let uncertain = Err(RuntimeError::InjectionFailed {
        expected: 12,
        sent: 4,
    });
    assert_eq!(
        undo_outcome_after_injection(&not_executed, true),
        UndoOutcome::NotExecuted
    );
    assert_eq!(
        undo_outcome_after_injection(&uncertain, true),
        UndoOutcome::Uncertain
    );
}

#[test]
fn certification_replacement_outcome_matches_actual_injection_effect() {
    let failed = Err(RuntimeError::InjectionFailed {
        expected: 12,
        sent: 4,
    });
    let succeeded: Result<(), RuntimeError> = Ok(());
    assert_eq!(
        replacement_outcome_after_injection(&failed, true),
        ReplacementOutcome::Aborted
    );
    assert_eq!(
        replacement_outcome_after_injection(&succeeded, true),
        ReplacementOutcome::Applied
    );
    assert_eq!(
        replacement_outcome_after_injection(&succeeded, false),
        ReplacementOutcome::Aborted
    );
}

#[test]
fn certification_failed_injection_never_suppresses_the_physical_keyup() {
    let failed = Err(RuntimeError::InjectionFailed {
        expected: 12,
        sent: 4,
    });
    assert_eq!(
        keyup_suppression_after_injection(VK_TAB as u32, &failed),
        None
    );
}
