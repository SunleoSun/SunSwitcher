use super::{Boundary, InputBuffer, InputEvent, InputOutcome};

#[test]
fn completes_token_on_space() {
    let mut buffer = InputBuffer::new();
    for character in "дял".chars() {
        assert_eq!(
            buffer.process(InputEvent::Character(character)),
            InputOutcome::Continue
        );
    }

    let InputOutcome::Completed(token) = buffer.process(InputEvent::Character(' ')) else {
        panic!("space should complete the token");
    };

    assert_eq!(token.text(), "дял");
    assert_eq!(token.boundary(), Boundary::Character(' '));
    assert!(buffer.current_token().is_empty());
}

#[test]
fn backspace_updates_the_owned_token_state() {
    let mut buffer = InputBuffer::new();
    for character in "дях".chars() {
        buffer.process(InputEvent::Character(character));
    }

    buffer.process(InputEvent::Backspace);
    buffer.process(InputEvent::Character('л'));

    assert_eq!(buffer.current_token(), "дял");
}

#[test]
fn invalidation_fails_closed_and_discards_stale_state() {
    let mut buffer = InputBuffer::new();
    for character in "дял".chars() {
        buffer.process(InputEvent::Character(character));
    }

    assert_eq!(
        buffer.process(InputEvent::Invalidate),
        InputOutcome::Invalidated
    );
    assert!(buffer.current_token().is_empty());
    assert_eq!(
        buffer.process(InputEvent::Character(' ')),
        InputOutcome::Continue
    );
}
