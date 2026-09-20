use crate::correction::{CorrectionDecision, ReplacementText};
use crate::input::{Boundary, CompletedToken};

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
}
