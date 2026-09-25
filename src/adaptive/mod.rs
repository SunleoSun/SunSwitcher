mod adaptive_runtime;

pub use adaptive_runtime::{
    AdaptiveCompletionSession, AdaptiveCorrectionDirective, AdaptiveCorrectionSession,
    AdaptiveLexicalRuntime, AdaptiveRuntimeError, LearningClient, LexicalSnapshotStore,
};

#[cfg(test)]
mod adaptive_runtime_certification;
#[cfg(test)]
mod adaptive_runtime_tests;
