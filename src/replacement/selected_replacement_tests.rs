use super::{
    SelectedReplacementEngine, SelectedReplacementText, SelectedText, SelectedTextDecision,
};

#[test]
fn plans_selected_text_replacement_from_typed_decision() {
    let selected = SelectedText::try_new("Hello world").unwrap();
    let decision =
        SelectedTextDecision::Replace(SelectedReplacementText::try_new("HELLO WORLD").unwrap());

    let action = SelectedReplacementEngine::new()
        .plan(selected, decision)
        .expect("selected replacement action");

    assert_eq!(action.source().as_str(), "Hello world");
    assert_eq!(action.replacement().as_str(), "HELLO WORLD");
}

#[test]
fn keep_and_identical_replacement_have_no_side_effect() {
    let engine = SelectedReplacementEngine::new();
    assert_eq!(
        engine.plan(
            SelectedText::try_new("text").unwrap(),
            SelectedTextDecision::Keep,
        ),
        None
    );
    assert_eq!(
        engine.plan(
            SelectedText::try_new("text").unwrap(),
            SelectedTextDecision::Replace(SelectedReplacementText::try_new("text").unwrap()),
        ),
        None
    );
}

#[test]
fn typed_selection_contract_rejects_unrepresentable_clipboard_text() {
    assert!(SelectedText::try_new("").is_err());
    assert!(SelectedText::try_new("a\0b").is_err());
    assert!(SelectedReplacementText::try_new("a\0b").is_err());
    assert!(SelectedReplacementText::try_new("").is_ok());
}
