use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VK_DELETE,
};

use super::keyboard_runtime::{build_selected_replacement_inputs, build_selected_text_inputs};
use super::selected_text_runtime::{
    ClipboardSnapshotMode, TrailingNonWhitespaceRange, caret_navigation_steps,
    classify_clipboard_snapshot, is_single_line_capture, replace_copied_chunk_range,
    trailing_non_whitespace_range, unicode_clipboard_units,
};

#[test]
fn trailing_non_whitespace_range_keeps_layout_punctuation_and_trailing_spaces() {
    assert_eq!(
        trailing_non_whitespace_range("abc lkz/  "),
        Some(TrailingNonWhitespaceRange {
            leading_chars: 4,
            selected_chars: 4,
            trailing_chars: 2,
        })
    );
    assert_eq!(
        trailing_non_whitespace_range("z"),
        Some(TrailingNonWhitespaceRange {
            leading_chars: 0,
            selected_chars: 1,
            trailing_chars: 0,
        })
    );
    assert_eq!(trailing_non_whitespace_range("   \t"), None);
}

#[test]
fn retained_previous_chunk_replacement_preserves_leading_text_and_suffix() {
    let range = TrailingNonWhitespaceRange {
        leading_chars: 4,
        selected_chars: 3,
        trailing_chars: 1,
    };
    assert_eq!(
        replace_copied_chunk_range("asd asd!", range, "фыв"),
        "asd фыв!"
    );
}

#[test]
fn copied_chunk_range_and_caret_steps_handle_line_breaks_without_losing_suffix() {
    assert!(!is_single_line_capture("привет привет.\r\n"));
    assert!(is_single_line_capture("привет привет."));
    assert_eq!(caret_navigation_steps("abc\r\ndef"), 7);
    assert_eq!(
        trailing_non_whitespace_range("привет привет."),
        Some(TrailingNonWhitespaceRange {
            leading_chars: 7,
            selected_chars: 7,
            trailing_chars: 0,
        })
    );
}

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
fn selected_replacement_batches_delete_before_unicode_text() {
    let text = "для";
    let inputs = build_selected_replacement_inputs(text);
    assert_eq!(inputs.len(), 2 + text.encode_utf16().count() * 2);

    let delete_down = unsafe { inputs[0].Anonymous.ki };
    let delete_up = unsafe { inputs[1].Anonymous.ki };
    assert_eq!(delete_down.wVk, VK_DELETE);
    assert_eq!(delete_down.dwFlags & KEYEVENTF_KEYUP, 0);
    assert_eq!(delete_up.wVk, VK_DELETE);
    assert_ne!(delete_up.dwFlags & KEYEVENTF_KEYUP, 0);

    for pair in inputs[2..].chunks_exact(2) {
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
