use super::{
    SelectedReplacementEngine, SelectedReplacementText, SelectedText, SelectedTextDecision,
};

#[test]
fn certification_selection_replacement_preserves_full_unicode_text() {
    let cases = [
        ("Привет, world!", "Hello, мир!"),
        ("line 1\nline 2\nline 3", "строка 1\nстрока 2\nстрока 3"),
        ("emoji 😀 + кириллица", "emoji 🚀 + English"),
        ("  spaces stay selected  ", "  spaces stay replaced  "),
    ];

    for (source, replacement) in cases {
        let action = SelectedReplacementEngine::new()
            .plan(
                SelectedText::try_new(source).unwrap(),
                SelectedTextDecision::Replace(
                    SelectedReplacementText::try_new(replacement).unwrap(),
                ),
            )
            .expect("non-identical selected replacement");

        assert_eq!(action.source().as_str(), source);
        assert_eq!(action.replacement().as_str(), replacement);
    }
}

#[test]
fn certification_empty_replacement_is_an_explicit_delete_not_invalid_state() {
    let action = SelectedReplacementEngine::new()
        .plan(
            SelectedText::try_new("delete me").unwrap(),
            SelectedTextDecision::Replace(SelectedReplacementText::try_new("").unwrap()),
        )
        .expect("empty replacement intentionally deletes selection");

    assert_eq!(action.source().as_str(), "delete me");
    assert_eq!(action.replacement().as_str(), "");
}
