mod adaptive_runtime;
mod live_prefix;

#[cfg(test)]
mod live_prefix_certification;

pub use adaptive_runtime::{
    AdaptiveCompletionSession, AdaptiveCorrectionDirective, AdaptiveCorrectionSession,
    AdaptiveLexicalRuntime, AdaptiveRuntimeError, LearningClient, LexicalSnapshotStore,
};

#[cfg(test)]
mod adaptive_runtime_certification;
#[cfg(test)]
mod adaptive_runtime_tests;
