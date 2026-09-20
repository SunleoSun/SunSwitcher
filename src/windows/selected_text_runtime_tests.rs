use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VK_DELETE,
};

use super::keyboard_runtime::build_selected_text_inputs;
use super::selected_text_runtime::{
    ClipboardSnapshotMode, classify_clipboard_snapshot, unicode_clipboard_units,
};

#[test]
fn unicode_clipboard_payload_is_nul_terminated() {
    assert_eq!(unicode_clipboard_units("abc"), vec![97, 98, 99, 0]);
}

#[test]
fn unicode_clipboard_payload_preserves_non_ascii_and_surrogate_pairs() {
    let units = unicode_clipboard_units("для 😀");
    assert_eq!(units.last(), Some(&0));
    assert_eq!(
        &units[..units.len() - 1],
        "для 😀".encode_utf16().collect::<Vec<_>>().as_slice()
    );
}

#[test]
fn clipboard_snapshot_policy_supports_empty_text_and_files() {
    assert_eq!(
        classify_clipboard_snapshot(0, false, false, false),
        ClipboardSnapshotMode::Empty
    );
    assert_eq!(
        classify_clipboard_snapshot(4, true, false, false),
        ClipboardSnapshotMode::UnicodeText
    );
    assert_eq!(
        classify_clipboard_snapshot(8, false, true, false),
        ClipboardSnapshotMode::FileDrop
    );
}

#[test]
fn copied_file_state_takes_priority_over_text_and_image_fallbacks() {
    assert_eq!(
        classify_clipboard_snapshot(8, true, true, true),
        ClipboardSnapshotMode::FileDrop
    );
}

#[test]
fn image_state_takes_priority_over_text_fallback() {
    assert_eq!(
        classify_clipboard_snapshot(6, true, false, true),
        ClipboardSnapshotMode::Image
    );
}

#[test]
fn clipboard_snapshot_policy_rejects_unknown_non_text_non_file_state() {
    assert_eq!(
        classify_clipboard_snapshot(1, false, false, false),
        ClipboardSnapshotMode::Unsupported
    );
}

#[test]
fn direct_selected_text_injection_uses_unicode_events() {
    let text = "для 😀";
    let inputs = build_selected_text_inputs(text);
    assert_eq!(inputs.len(), text.encode_utf16().count() * 2);

    for pair in inputs.chunks_exact(2) {
        let down = unsafe { pair[0].Anonymous.ki };
        let up = unsafe { pair[1].Anonymous.ki };
        assert_eq!(down.wVk, 0);
        assert_ne!(down.dwFlags & KEYEVENTF_UNICODE, 0);
        assert_eq!(down.dwFlags & KEYEVENTF_KEYUP, 0);
        assert_eq!(up.wVk, 0);
        assert_ne!(up.dwFlags & KEYEVENTF_UNICODE, 0);
        assert_ne!(up.dwFlags & KEYEVENTF_KEYUP, 0);
    }
}

#[test]
fn empty_selected_replacement_deletes_the_active_selection() {
    let inputs = build_selected_text_inputs("");
    assert_eq!(inputs.len(), 2);
    let down = unsafe { inputs[0].Anonymous.ki };
    let up = unsafe { inputs[1].Anonymous.ki };
    assert_eq!(down.wVk, VK_DELETE);
    assert_eq!(down.dwFlags & KEYEVENTF_KEYUP, 0);
    assert_eq!(up.wVk, VK_DELETE);
    assert_ne!(up.dwFlags & KEYEVENTF_KEYUP, 0);
}
