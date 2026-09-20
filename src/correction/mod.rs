mod correction_engine;

pub use correction_engine::{
    Confidence, ConfidenceError, CorrectionCandidate, CorrectionCandidateProvider,
    CorrectionDecision, CorrectionEngine, ReplacementText, ReplacementTextError,
};

#[cfg(test)]
mod correction_engine_certification;
#[cfg(test)]
mod correction_engine_tests;
