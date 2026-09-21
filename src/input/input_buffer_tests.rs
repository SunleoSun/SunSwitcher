use super::{Boundary, InputBuffer, InputEvent, InputOutcome, PhysicalKey};

#[test]
fn completes_token_on_space() {
    let mut buffer = InputBuffer::new();
    for character in "дял".chars() {
        assert_eq!(
            buffer.process(InputEvent::character(character)),
            InputOutcome::Continue
        );
    }

    let InputOutcome::Completed(token) = buffer.process(InputEvent::character(' ')) else {
        panic!("space should complete the token");
    };

    assert_eq!(token.text(), "дял");
    assert_eq!(token.boundary(), Boundary::Character(' '));
    assert!(
        token
            .physical_keys()
            .iter()
            .all(|key| *key == PhysicalKey::Other)
    );
    assert!(buffer.current_token().is_empty());
}

#[test]
fn backspace_updates_text_and_physical_key_state_together() {
    let mut buffer = InputBuffer::new();
    buffer.process(InputEvent::typed_character(';', PhysicalKey::Semicolon));
    buffer.process(InputEvent::character('x'));
    buffer.process(InputEvent::Backspace);

    let InputOutcome::Completed(token) = buffer.process(InputEvent::character(' ')) else {
        panic!("expected completion");
    };
    assert_eq!(token.text(), ";");
    assert_eq!(token.physical_keys(), &[PhysicalKey::Semicolon]);
}

#[test]
fn plain_punctuation_is_a_boundary_but_layout_ambiguous_physical_key_is_deferred() {
    let mut plain = InputBuffer::new();
    plain.process(InputEvent::character('a'));
    let InputOutcome::Completed(token) = plain.process(InputEvent::character(',')) else {
        panic!("plain comma should complete the token");
    };
    assert_eq!(token.text(), "a");
    assert_eq!(token.boundary(), Boundary::Character(','));

    let mut physical = InputBuffer::new();
    physical.process(InputEvent::character('a'));
    assert_eq!(
        physical.process(InputEvent::typed_character(',', PhysicalKey::Comma)),
        InputOutcome::Continue
    );
    assert_eq!(physical.current_token(), "a,");
}

#[test]
fn invalidation_discards_old_state_but_first_fresh_character_restarts_tracking() {
    let mut buffer = InputBuffer::new();
    buffer.process(InputEvent::typed_character('[', PhysicalKey::LeftBracket));
    buffer.process(InputEvent::character('a'));

    assert_eq!(
        buffer.process(InputEvent::Invalidate),
        InputOutcome::Invalidated
    );
    assert!(buffer.current_token().is_empty());

    // Backspace cannot establish ownership because it may edit text that predates invalidation.
    assert_eq!(
        buffer.process(InputEvent::Backspace),
        InputOutcome::Continue
    );
    assert!(buffer.current_token().is_empty());

    buffer.process(InputEvent::character('x'));
    buffer.process(InputEvent::typed_character(',', PhysicalKey::Comma));
    assert_eq!(buffer.current_token(), "x,");

    let InputOutcome::Completed(token) = buffer.process(InputEvent::character(' ')) else {
        panic!("freshly typed text should complete normally after invalidation");
    };
    assert_eq!(token.text(), "x,");
}

#[test]
fn physical_key_recognizes_base_and_shifted_oem_symbols() {
    for (character, expected) in [
        ('`', PhysicalKey::Grave),
        ('~', PhysicalKey::Grave),
        ('[', PhysicalKey::LeftBracket),
        ('{', PhysicalKey::LeftBracket),
        (']', PhysicalKey::RightBracket),
        ('}', PhysicalKey::RightBracket),
        (';', PhysicalKey::Semicolon),
        (':', PhysicalKey::Semicolon),
        ('\'', PhysicalKey::Quote),
        ('"', PhysicalKey::Quote),
        (',', PhysicalKey::Comma),
        ('<', PhysicalKey::Comma),
        ('.', PhysicalKey::Period),
        ('>', PhysicalKey::Period),
    ] {
        assert_eq!(PhysicalKey::from_layout_symbol(character), expected);
    }
}
