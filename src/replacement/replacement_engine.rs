use crate::correction::{CorrectionDecision, ReplacementText};
use crate::input::{Boundary, CompletedToken};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementOutcome {
    Applied,
    Aborted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoOutcome {
    Applied,
    NotExecuted,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacementAction {
    delete_previous_chars: usize,
    replacement: ReplacementText,
    boundary: Boundary,
}

impl ReplacementAction {
    pub fn delete_previous_chars(&self) -> usize {
        self.delete_previous_chars
    }

    pub fn replacement(&self) -> &ReplacementText {
        &self.replacement
    }

    pub fn boundary(&self) -> Boundary {
        self.boundary
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ReplacementEngine;

impl ReplacementEngine {
    pub fn new() -> Self {
        Self
    }

    pub fn plan(
        &self,
        token: &CompletedToken,
        decision: CorrectionDecision,
    ) -> Option<ReplacementAction> {
        let CorrectionDecision::Replace(replacement) = decision else {
            return None;
        };

        Some(ReplacementAction {
            delete_previous_chars: token.text().chars().count(),
            replacement,
            boundary: token.boundary(),
        })
    }

    pub fn plan_immediate_undo(
        &self,
        original_text: &str,
        applied: &ReplacementAction,
    ) -> Option<ReplacementAction> {
        let Boundary::Character(boundary) = applied.boundary() else {
            return None;
        };
        let original = ReplacementText::try_new(original_text).ok()?;
        Some(ReplacementAction {
            delete_previous_chars: applied.replacement().as_str().chars().count() + 1,
            replacement: original,
            boundary: Boundary::Character(boundary),
        })
    }
}
