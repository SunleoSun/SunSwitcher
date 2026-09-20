mod correction_engine;
mod lexical_provider;

pub use correction_engine::{
    Confidence, ConfidenceError, CorrectionCandidate, CorrectionCandidateProvider,
    CorrectionDecision, CorrectionEngine, ReplacementText, ReplacementTextError,
};
pub use lexical_provider::{LexicalCorrectionProvider, LexicalProviderError};

#[cfg(test)]
mod correction_engine_certification;
#[cfg(test)]
mod correction_engine_tests;
#[cfg(test)]
mod lexical_provider_certification;
#[cfg(test)]
mod lexical_provider_tests;
