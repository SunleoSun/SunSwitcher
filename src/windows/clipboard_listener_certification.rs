use super::clipboard_listener::{should_deliver_clipboard_read, should_observe_clipboard_sequence};

#[test]
fn certification_clipboard_listener_ignores_startup_and_internal_mutations_but_observes_user_changes()
 {
    assert!(!should_observe_clipboard_sequence(10, 10, 0, 0));
    assert!(should_observe_clipboard_sequence(10, 11, 0, 0));
    assert!(!should_observe_clipboard_sequence(11, 12, 1, 0));
    assert!(!should_observe_clipboard_sequence(12, 13, 0, 13));
    assert!(should_observe_clipboard_sequence(13, 14, 0, 13));
}

#[test]
fn certification_clipboard_listener_delivers_only_a_stable_user_owned_read() {
    assert!(should_deliver_clipboard_read(20, 20, 0, 0));
    assert!(!should_deliver_clipboard_read(20, 21, 0, 0));
    assert!(!should_deliver_clipboard_read(20, 20, 1, 0));
    assert!(!should_deliver_clipboard_read(20, 20, 0, 20));
}
