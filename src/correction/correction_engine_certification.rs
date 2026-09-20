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
fn certification_candidate_order_does_not_change_the_best_decision() {
    let token = CompletedToken::new("дял", Boundary::Character(' '));
    let threshold = Confidence::try_new(0.75).unwrap();

    for candidates in [
        vec![candidate("дал", 0.80), candidate("для", 0.99)],
        vec![candidate("для", 0.99), candidate("дал", 0.80)],
    ] {
        let engine = CorrectionEngine::new(Provider(candidates), threshold);
        let CorrectionDecision::Replace(replacement) = engine.decide(&token) else {
            panic!("expected replacement");
        };
        assert_eq!(replacement.as_str(), "для");
    }
}

#[test]
fn certification_never_replaces_a_token_with_identical_text() {
    let token = CompletedToken::new("для", Boundary::Character(' '));
    let engine = CorrectionEngine::new(
        Provider(vec![candidate("для", 1.0)]),
        Confidence::try_new(0.50).unwrap(),
    );

    assert_eq!(engine.decide(&token), CorrectionDecision::Keep);
}

#[test]
fn certification_empty_candidate_set_is_a_safe_keep() {
    let token = CompletedToken::new("неизвестное", Boundary::Character(' '));
    let engine = CorrectionEngine::new(Provider(Vec::new()), Confidence::try_new(0.50).unwrap());

    assert_eq!(engine.decide(&token), CorrectionDecision::Keep);
}
