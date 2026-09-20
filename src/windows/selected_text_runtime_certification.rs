use windows_sys::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_UNICODE;

use super::keyboard_runtime::build_selected_text_inputs;
use super::selected_text_runtime::{
    ClipboardSnapshotMode, classify_clipboard_snapshot, unicode_clipboard_units,
};

#[test]
fn certification_clipboard_payload_round_trips_representative_selected_text() {
    let cases = [
        "Short selection",
        "Русский текст с punctuation!",
        "line 1\r\nline 2\r\nline 3",
        "emoji 😀🚀 and symbols → ✓",
        "A long selection can be processed without using the clipboard as the replacement transport.",
    ];

    for text in cases {
        let units = unicode_clipboard_units(text);
        assert_eq!(units.last(), Some(&0));
        let decoded = String::from_utf16(&units[..units.len() - 1]).unwrap();
        assert_eq!(decoded, text);
    }
}

#[test]
fn certification_plain_text_clipboard_is_supported_even_with_auxiliary_formats() {
    for format_count in [1, 2, 4, 8, 16] {
        assert_eq!(
            classify_clipboard_snapshot(format_count, true, false, false),
            ClipboardSnapshotMode::UnicodeText
        );
    }
}

#[test]
fn certification_copied_file_clipboard_is_a_supported_preservation_state() {
    for format_count in [1, 2, 4, 8, 16] {
        assert_eq!(
            classify_clipboard_snapshot(format_count, false, true, false),
            ClipboardSnapshotMode::FileDrop
        );
    }
}

#[test]
fn certification_file_clipboard_wins_when_windows_also_exposes_text_or_image() {
    assert_eq!(
        classify_clipboard_snapshot(8, true, true, true),
        ClipboardSnapshotMode::FileDrop
    );
}

#[test]
fn certification_screenshot_image_clipboard_is_supported() {
    for format_count in [1, 2, 4, 8] {
        assert_eq!(
            classify_clipboard_snapshot(format_count, false, false, true),
            ClipboardSnapshotMode::Image
        );
    }
}

#[test]
fn certification_image_clipboard_wins_over_text_fallback() {
    assert_eq!(
        classify_clipboard_snapshot(6, true, false, true),
        ClipboardSnapshotMode::Image
    );
}

#[test]
fn certification_unknown_non_text_non_file_clipboard_fails_closed() {
    for format_count in [1, 2, 8] {
        assert_eq!(
            classify_clipboard_snapshot(format_count, false, false, false),
            ClipboardSnapshotMode::Unsupported
        );
    }
}

#[test]
fn certification_large_selected_replacement_is_one_direct_input_batch() {
    let text = format!("{}{}", "абвгд".repeat(600), " 😀 end");
    let inputs = build_selected_text_inputs(&text);
    assert_eq!(inputs.len(), text.encode_utf16().count() * 2);
    assert!(inputs.iter().all(|input| {
        let keyboard = unsafe { input.Anonymous.ki };
        keyboard.wVk == 0 && keyboard.dwFlags & KEYEVENTF_UNICODE != 0
    }));
}
