use std::cmp::Ordering;

use crate::input::CompletedToken;
use crate::language::LanguageId;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrectionReplacement {
    text: ReplacementText,
    target_language: Option<LanguageId>,
}

impl CorrectionReplacement {
    pub fn new(text: ReplacementText) -> Self {
        Self {
            text,
            target_language: None,
        }
    }

    pub fn for_language(text: ReplacementText, target_language: LanguageId) -> Self {
        Self {
            text,
            target_language: Some(target_language),
        }
    }

    pub fn as_str(&self) -> &str {
        self.text.as_str()
    }

    pub fn target_language(&self) -> Option<&LanguageId> {
        self.target_language.as_ref()
    }

    pub fn into_parts(self) -> (ReplacementText, Option<LanguageId>) {
        (self.text, self.target_language)
    }
}

impl From<ReplacementText> for CorrectionReplacement {
    fn from(text: ReplacementText) -> Self {
        Self::new(text)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorrectionFeatures {
    typo: bool,
    layout_switch: bool,
}

impl CorrectionFeatures {
    pub const fn new(typo: bool, layout_switch: bool) -> Self {
        Self {
            typo,
            layout_switch,
        }
    }

    pub const fn typo(self) -> bool {
        self.typo
    }

    pub const fn layout_switch(self) -> bool {
        self.layout_switch
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorrectionFeaturePolicy {
    autocorrections: bool,
    auto_keyboard_switches: bool,
}

impl CorrectionFeaturePolicy {
    pub const ALL: Self = Self::new(true, true);

    pub const fn new(autocorrections: bool, auto_keyboard_switches: bool) -> Self {
        Self {
            autocorrections,
            auto_keyboard_switches,
        }
    }

    pub const fn auto_keyboard_switches(self) -> bool {
        self.auto_keyboard_switches
    }

    pub const fn allows(self, features: CorrectionFeatures) -> bool {
        (!features.typo() || self.autocorrections)
            && (!features.layout_switch() || self.auto_keyboard_switches)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CorrectionCandidate {
    replacement: CorrectionReplacement,
    confidence: Confidence,
    features: CorrectionFeatures,
}

impl CorrectionCandidate {
    pub fn new(replacement: ReplacementText, confidence: Confidence) -> Self {
        Self {
            replacement: replacement.into(),
            confidence,
            features: CorrectionFeatures::new(true, false),
        }
    }

    pub fn for_language(
        replacement: ReplacementText,
        confidence: Confidence,
        target_language: LanguageId,
    ) -> Self {
        Self {
            replacement: CorrectionReplacement::for_language(replacement, target_language),
            confidence,
            features: CorrectionFeatures::new(false, true),
        }
    }

    pub fn classified(
        replacement: ReplacementText,
        confidence: Confidence,
        target_language: Option<LanguageId>,
        features: CorrectionFeatures,
    ) -> Self {
        let replacement = match target_language {
            Some(language) => CorrectionReplacement::for_language(replacement, language),
            None => CorrectionReplacement::new(replacement),
        };
        Self {
            replacement,
            confidence,
            features,
        }
    }

    pub fn replacement(&self) -> &ReplacementText {
        &self.replacement.text
    }

    pub fn target_language(&self) -> Option<&LanguageId> {
        self.replacement.target_language()
    }

    pub fn confidence(&self) -> Confidence {
        self.confidence
    }

    pub fn features(&self) -> CorrectionFeatures {
        self.features
    }
}

pub trait CorrectionCandidateProvider {
    fn candidates(&self, token: &CompletedToken) -> Vec<CorrectionCandidate>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorrectionDecision {
    Keep,
    Replace(CorrectionReplacement),
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
        self.decide_with_policy(token, CorrectionFeaturePolicy::ALL)
    }

    pub fn decide_with_policy(
        &self,
        token: &CompletedToken,
        policy: CorrectionFeaturePolicy,
    ) -> CorrectionDecision {
        let best = self
            .provider
            .candidates(token)
            .into_iter()
            .filter(|candidate| policy.allows(candidate.features()))
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
