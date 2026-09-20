use super::{Boundary, InputBuffer, InputEvent, InputOutcome};

fn complete(text: &str, boundary_event: InputEvent) -> (String, Boundary) {
    let mut buffer = InputBuffer::new();
    for character in text.chars() {
        assert_eq!(
            buffer.process(InputEvent::Character(character)),
            InputOutcome::Continue
        );
    }

    let InputOutcome::Completed(token) = buffer.process(boundary_event) else {
        panic!("expected a completed token for {text:?}");
    };
    (token.text().to_owned(), token.boundary())
}

#[test]
fn certification_common_boundaries_complete_exactly_the_owned_token() {
    let cases = [
        (InputEvent::Character(' '), Boundary::Character(' ')),
        (InputEvent::Character('.'), Boundary::Character('.')),
        (InputEvent::Character(','), Boundary::Character(',')),
        (InputEvent::Character('!'), Boundary::Character('!')),
        (InputEvent::Character('?'), Boundary::Character('?')),
        (InputEvent::Character(')'), Boundary::Character(')')),
        (InputEvent::Boundary(Boundary::Enter), Boundary::Enter),
        (InputEvent::Boundary(Boundary::Tab), Boundary::Tab),
    ];

    for (event, expected_boundary) in cases {
        let (token, boundary) = complete("дял", event);
        assert_eq!(token, "дял");
        assert_eq!(boundary, expected_boundary);
    }
}

#[test]
fn certification_edit_then_complete_reports_the_final_visible_token() {
    let mut buffer = InputBuffer::new();
    for character in "дях".chars() {
        buffer.process(InputEvent::Character(character));
    }
    buffer.process(InputEvent::Backspace);
    buffer.process(InputEvent::Character('л'));

    let InputOutcome::Completed(token) = buffer.process(InputEvent::Character(' ')) else {
        panic!("expected completion");
    };
    assert_eq!(token.text(), "дял");
}

#[test]
fn certification_cursor_or_unknown_editing_action_prevents_stale_completion() {
    let mut buffer = InputBuffer::new();
    for character in "дял".chars() {
        buffer.process(InputEvent::Character(character));
    }
    assert_eq!(
        buffer.process(InputEvent::Invalidate),
        InputOutcome::Invalidated
    );

    assert_eq!(
        buffer.process(InputEvent::Character(' ')),
        InputOutcome::Continue
    );
    assert!(buffer.current_token().is_empty());
}
