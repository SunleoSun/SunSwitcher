use std::cmp::Ordering;

use crate::input::CompletedToken;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacementText(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplacementTextError;

impl ReplacementText {
    pub fn try_new(value: impl Into<String>) -> Result<Self, ReplacementTextError> {
        let value = value.into();
        if value.is_empty() || value.contains('\0') {
            return Err(ReplacementTextError);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Confidence(f32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfidenceError;

impl Confidence {
    pub const CERTAIN: Self = Self(1.0);

    pub fn try_new(value: f32) -> Result<Self, ConfidenceError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(ConfidenceError);
        }
        Ok(Self(value))
    }

    pub fn value(self) -> f32 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CorrectionCandidate {
    replacement: ReplacementText,
    confidence: Confidence,
}

impl CorrectionCandidate {
    pub fn new(replacement: ReplacementText, confidence: Confidence) -> Self {
        Self {
            replacement,
            confidence,
        }
    }

    pub fn replacement(&self) -> &ReplacementText {
        &self.replacement
    }

    pub fn confidence(&self) -> Confidence {
        self.confidence
    }
}

pub trait CorrectionCandidateProvider {
    fn candidates(&self, token: &CompletedToken) -> Vec<CorrectionCandidate>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorrectionDecision {
    Keep,
    Replace(ReplacementText),
}

pub struct CorrectionEngine<P> {
    provider: P,
    minimum_confidence: Confidence,
}

impl<P: CorrectionCandidateProvider> CorrectionEngine<P> {
    pub fn new(provider: P, minimum_confidence: Confidence) -> Self {
        Self {
            provider,
            minimum_confidence,
        }
    }

    pub fn decide(&self, token: &CompletedToken) -> CorrectionDecision {
        let best = self
            .provider
            .candidates(token)
            .into_iter()
            .filter(|candidate| candidate.replacement().as_str() != token.text())
            .filter(|candidate| candidate.confidence().value() >= self.minimum_confidence.value())
            .max_by(|left, right| {
                left.confidence()
                    .value()
                    .partial_cmp(&right.confidence().value())
                    .unwrap_or(Ordering::Equal)
            });

        match best {
            Some(candidate) => CorrectionDecision::Replace(candidate.replacement),
            None => CorrectionDecision::Keep,
        }
    }
}
