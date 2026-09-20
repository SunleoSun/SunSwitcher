use super::{
    Confidence, CorrectionCandidate, CorrectionCandidateProvider, CorrectionDecision,
    CorrectionEngine, ReplacementText,
};
use crate::input::{Boundary, CompletedToken};

struct Provider(Vec<CorrectionCandidate>);

impl CorrectionCandidateProvider for Provider {
    fn candidates(&self, _token: &CompletedToken) -> Vec<CorrectionCandidate> {
        self.0.clone()
    }
}

fn candidate(text: &str, confidence: f32) -> CorrectionCandidate {
    CorrectionCandidate::new(
        ReplacementText::try_new(text).unwrap(),
        Confidence::try_new(confidence).unwrap(),
    )
}

#[test]
fn selects_the_highest_confidence_candidate_above_threshold() {
    let engine = CorrectionEngine::new(
        Provider(vec![candidate("дал", 0.80), candidate("для", 0.99)]),
        Confidence::try_new(0.90).unwrap(),
    );
    let token = CompletedToken::new("дял", Boundary::Character(' '));

    let CorrectionDecision::Replace(replacement) = engine.decide(&token) else {
        panic!("expected replacement");
    };
    assert_eq!(replacement.as_str(), "для");
}

#[test]
fn keeps_token_when_no_candidate_reaches_threshold() {
    let engine = CorrectionEngine::new(
        Provider(vec![candidate("для", 0.70)]),
        Confidence::try_new(0.90).unwrap(),
    );
    let token = CompletedToken::new("дял", Boundary::Character(' '));

    assert_eq!(engine.decide(&token), CorrectionDecision::Keep);
}

#[test]
fn typed_value_contracts_reject_invalid_values() {
    assert!(ReplacementText::try_new("").is_err());
    assert!(ReplacementText::try_new("a\0b").is_err());
    assert!(Confidence::try_new(-0.1).is_err());
    assert!(Confidence::try_new(1.1).is_err());
    assert!(Confidence::try_new(f32::NAN).is_err());
}
