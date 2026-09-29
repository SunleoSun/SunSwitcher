mod replacement_engine;
mod selected_replacement;

pub use replacement_engine::{
    LivePrefixReplacementAction, ReplacementAction, ReplacementEngine, ReplacementOutcome,
    UndoOutcome, UndoReplacementAction,
};
pub use selected_replacement::{
    SelectedReplacementAction, SelectedReplacementEngine, SelectedReplacementText,
    SelectedReplacementTextError, SelectedText, SelectedTextDecision, SelectedTextError,
};

#[cfg(test)]
mod replacement_engine_certification;
#[cfg(test)]
mod replacement_engine_tests;
#[cfg(test)]
mod selected_replacement_certification;
#[cfg(test)]
mod selected_replacement_tests;
