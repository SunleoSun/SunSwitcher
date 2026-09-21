use super::ReplacementEngine;
use crate::correction::{CorrectionDecision, ReplacementText};
use crate::input::{Boundary, CompletedToken};

#[test]
fn certification_delete_count_uses_unicode_characters_not_utf8_bytes() {
    let engine = ReplacementEngine::new();
    let cases = [
        ("abcx", "ABC_REPLACED", 4),
        ("дял", "для", 3),
        ("тчо", "что", 3),
    ];

    for (source, replacement, expected_delete_count) in cases {
        let token = CompletedToken::new(source, Boundary::Character('.'));
        let action = engine
            .plan(
                &token,
                CorrectionDecision::Replace(ReplacementText::try_new(replacement).unwrap()),
            )
            .expect("replacement action");
        assert_eq!(action.delete_previous_chars(), expected_delete_count);
        assert_eq!(action.replacement().as_str(), replacement);
        assert_eq!(action.boundary(), Boundary::Character('.'));
    }
}

#[test]
fn certification_immediate_undo_preserves_original_text_and_boundary() {
    let engine = ReplacementEngine::new();
    let token = CompletedToken::new("QuanntileEntrySrtategy", Boundary::Character(' '));
    let applied = engine
        .plan(
            &token,
            CorrectionDecision::Replace(ReplacementText::try_new("QuantileEntryStrategy").unwrap()),
        )
        .unwrap();
    let undo = engine
        .plan_immediate_undo("QuanntileEntrySrtategy", &applied)
        .unwrap();

    assert_eq!(
        undo.delete_previous_chars(),
        "QuantileEntryStrategy".chars().count() + 1
    );
    assert_eq!(undo.replacement().as_str(), "QuanntileEntrySrtategy");
    assert_eq!(undo.boundary(), Boundary::Character(' '));
}

#[test]
fn certification_boundary_semantics_are_preserved_for_execution() {
    let engine = ReplacementEngine::new();
    for boundary in [Boundary::Character('!'), Boundary::Enter, Boundary::Tab] {
        let token = CompletedToken::new("дял", boundary);
        let action = engine
            .plan(
                &token,
                CorrectionDecision::Replace(ReplacementText::try_new("для").unwrap()),
            )
            .expect("replacement action");
        assert_eq!(action.boundary(), boundary);
    }
}
