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
fn immediate_undo_reverses_only_character_boundary_replacements() {
    let engine = ReplacementEngine::new();
    let token = CompletedToken::new("дял", Boundary::Character(' '));
    let applied = engine
        .plan(
            &token,
            CorrectionDecision::Replace(ReplacementText::try_new("для").unwrap()),
        )
        .unwrap();

    let undo = engine
        .plan_immediate_undo("дял", &applied)
        .expect("space-delimited replacement is immediately undoable");
    assert_eq!(undo.delete_previous_chars(), 4);
    assert_eq!(undo.replacement().as_str(), "дял");
    assert_eq!(undo.boundary(), Boundary::Character(' '));

    for boundary in [Boundary::Enter, Boundary::Tab] {
        let token = CompletedToken::new("дял", boundary);
        let applied = engine
            .plan(
                &token,
                CorrectionDecision::Replace(ReplacementText::try_new("для").unwrap()),
            )
            .unwrap();
        assert_eq!(engine.plan_immediate_undo("дял", &applied), None);
    }
}

#[test]
fn keep_has_no_replacement_side_effect() {
    let engine = ReplacementEngine::new();
    let token = CompletedToken::new("для", Boundary::Character(' '));

    assert_eq!(engine.plan(&token, CorrectionDecision::Keep), None);
}
