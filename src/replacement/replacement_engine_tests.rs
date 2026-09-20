use super::ReplacementEngine;
use crate::correction::{CorrectionDecision, ReplacementText};
use crate::input::{Boundary, CompletedToken};

#[test]
fn plans_delete_insert_and_boundary_from_typed_decision() {
    let engine = ReplacementEngine::new();
    let token = CompletedToken::new("дял", Boundary::Character(' '));
    let decision = CorrectionDecision::Replace(ReplacementText::try_new("для").unwrap());

    let action = engine.plan(&token, decision).expect("replacement action");
    assert_eq!(action.delete_previous_chars(), 3);
    assert_eq!(action.replacement().as_str(), "для");
    assert_eq!(action.boundary(), Boundary::Character(' '));
}

#[test]
fn keep_has_no_replacement_side_effect() {
    let engine = ReplacementEngine::new();
    let token = CompletedToken::new("для", Boundary::Character(' '));

    assert_eq!(engine.plan(&token, CorrectionDecision::Keep), None);
}
