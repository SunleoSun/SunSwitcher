use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VK_BACK, VK_RETURN, VK_TAB,
};

use super::keyboard_runtime::{
    KeyDownDisposition, RuntimeError, build_replacement_inputs,
    foreground_change_requires_invalidation, keyup_suppression_after_injection,
    preclassify_key_down,
};
use crate::correction::{CorrectionDecision, ReplacementText};
use crate::input::{Boundary, CompletedToken, InputBuffer, InputEvent, InputOutcome};
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
fn certification_command_modified_editing_keys_fail_closed() {
    for vk_code in [VK_BACK, VK_RETURN, VK_TAB] {
        assert_eq!(
            preclassify_key_down(vk_code as u32, true),
            KeyDownDisposition::Event(InputEvent::Invalidate)
        );
    }
}

#[test]
fn certification_foreground_change_discards_stale_token_before_next_boundary() {
    let mut buffer = InputBuffer::new();
    for character in "дял".chars() {
        assert_eq!(
            buffer.process(InputEvent::Character(character)),
            InputOutcome::Continue
        );
    }

    assert!(foreground_change_requires_invalidation(100, 200));
    assert_eq!(
        buffer.process(InputEvent::Invalidate),
        InputOutcome::Invalidated
    );
    assert_eq!(
        buffer.process(InputEvent::Character(' ')),
        InputOutcome::Continue
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
