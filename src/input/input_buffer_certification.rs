use super::{Boundary, InputBuffer, InputEvent, InputOutcome, PhysicalKey};

fn complete(text: &str, boundary_event: InputEvent) -> (String, Boundary) {
    let mut buffer = InputBuffer::new();
    for character in text.chars() {
        assert_eq!(
            buffer.process(InputEvent::character(character)),
            InputOutcome::Continue
        );
    }

    let InputOutcome::Completed(token) = buffer.process(boundary_event) else {
        panic!("expected a completed token for {text:?}");
    };
    (token.text().to_owned(), token.boundary())
}

#[test]
fn certification_common_unambiguous_boundaries_complete_exactly_the_owned_token() {
    let cases = [
        (InputEvent::character(' '), Boundary::Character(' ')),
        (InputEvent::character('.'), Boundary::Character('.')),
        (InputEvent::character(','), Boundary::Character(',')),
        (InputEvent::character('!'), Boundary::Character('!')),
        (InputEvent::character('?'), Boundary::Character('?')),
        (InputEvent::character(')'), Boundary::Character(')')),
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
fn certification_all_layout_ambiguous_oem_keys_can_remain_in_a_potential_token() {
    let cases = [
        ('`', PhysicalKey::Grave),
        ('[', PhysicalKey::LeftBracket),
        (']', PhysicalKey::RightBracket),
        (';', PhysicalKey::Semicolon),
        ('\'', PhysicalKey::Quote),
        (',', PhysicalKey::Comma),
        ('.', PhysicalKey::Period),
        ('~', PhysicalKey::Grave),
        ('{', PhysicalKey::LeftBracket),
        ('}', PhysicalKey::RightBracket),
        (':', PhysicalKey::Semicolon),
        ('"', PhysicalKey::Quote),
        ('<', PhysicalKey::Comma),
        ('>', PhysicalKey::Period),
    ];

    for (produced, key) in cases {
        let mut buffer = InputBuffer::new();
        assert_eq!(
            buffer.process(InputEvent::typed_character(produced, key)),
            InputOutcome::Continue
        );
        let InputOutcome::Completed(token) = buffer.process(InputEvent::character(' ')) else {
            panic!("ambiguous key should be deferred until the unambiguous boundary");
        };
        assert_eq!(token.text(), produced.to_string());
        assert_eq!(token.physical_keys(), &[key]);
    }
}

#[test]
fn certification_edit_then_complete_reports_the_final_visible_token() {
    let mut buffer = InputBuffer::new();
    for character in "дях".chars() {
        buffer.process(InputEvent::character(character));
    }
    buffer.process(InputEvent::Backspace);
    buffer.process(InputEvent::character('л'));

    let InputOutcome::Completed(token) = buffer.process(InputEvent::character(' ')) else {
        panic!("expected completion");
    };
    assert_eq!(token.text(), "дял");
}

#[test]
fn certification_cursor_or_unknown_editing_action_prevents_stale_completion() {
    let mut buffer = InputBuffer::new();
    for character in "дял".chars() {
        buffer.process(InputEvent::character(character));
    }
    assert_eq!(
        buffer.process(InputEvent::Invalidate),
        InputOutcome::Invalidated
    );

    // Text typed while the caret context is unknown must not become a partial owned token.
    for character in "привте".chars() {
        assert_eq!(
            buffer.process(InputEvent::character(character)),
            InputOutcome::Continue
        );
    }
    assert!(buffer.current_token().is_empty());

    // The first unambiguous boundary re-establishes a known token start for subsequent input.
    assert_eq!(
        buffer.process(InputEvent::character(' ')),
        InputOutcome::Continue
    );
    for character in "дял".chars() {
        assert_eq!(
            buffer.process(InputEvent::character(character)),
            InputOutcome::Continue
        );
    }
    let InputOutcome::Completed(token) = buffer.process(InputEvent::character(' ')) else {
        panic!("expected tracking to resume after an unambiguous boundary");
    };
    assert_eq!(token.text(), "дял");
}
