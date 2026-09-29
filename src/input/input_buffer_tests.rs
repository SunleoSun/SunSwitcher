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
    assert_eq!(plain.current_layout_span(), "a,");
    assert_eq!(
        plain.process(InputEvent::character('!')),
        InputOutcome::Continue
    );
    assert_eq!(plain.current_layout_span(), "a,!");
    assert_eq!(
        plain.process(InputEvent::character(' ')),
        InputOutcome::Continue
    );
    assert_eq!(plain.current_layout_span(), "a,! ");
    assert_eq!(
        plain.process(InputEvent::character('x')),
        InputOutcome::Continue
    );
    assert_eq!(plain.current_layout_span(), "x");

    let mut physical = InputBuffer::new();
    physical.process(InputEvent::character('a'));
    assert_eq!(
        physical.process(InputEvent::typed_character(',', PhysicalKey::Comma)),
        InputOutcome::Continue
    );
    assert_eq!(physical.current_token(), "a,");
    assert_eq!(physical.current_layout_span(), "a,");
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
fn invalidation_after_layout_switch_replacement_drops_layout_span_ownership() {
    let mut buffer = InputBuffer::new();
    buffer.replace_layout_switch_span("как");
    assert_eq!(buffer.current_layout_span(), "как");

    assert_eq!(
        buffer.process(InputEvent::Invalidate),
        InputOutcome::Invalidated
    );
    assert!(buffer.current_layout_span().is_empty());
    assert!(buffer.current_token().is_empty());
}

#[test]
fn explicit_layout_switch_replacement_remains_owned_without_becoming_a_correction_token() {
    let mut buffer = InputBuffer::new();
    for character in "привет".chars() {
        buffer.process(InputEvent::character(character));
    }

    buffer.replace_layout_switch_span("ghbdtn ");
    assert_eq!(buffer.current_layout_span(), "ghbdtn ");
    assert!(buffer.current_token().is_empty());

    assert_eq!(
        buffer.process(InputEvent::Backspace),
        InputOutcome::Continue
    );
    assert_eq!(buffer.current_layout_span(), "ghbdtn");
    assert!(buffer.current_token().is_empty());

    assert_eq!(
        buffer.process(InputEvent::character('x')),
        InputOutcome::Continue
    );
    assert_eq!(buffer.current_layout_span(), "ghbdtnx");
    assert_eq!(buffer.current_token(), "x");
}

#[test]
fn physical_key_recognizes_base_and_shifted_oem_symbols() {
    for (character, expected) in [
        ('&', PhysicalKey::Digit7),
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
