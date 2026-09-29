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
    target_language: Option<crate::language::LanguageId>,
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

    pub fn target_language(&self) -> Option<&crate::language::LanguageId> {
        self.target_language.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivePrefixReplacementAction {
    delete_previous_chars: usize,
    replacement: ReplacementText,
    original_visible: String,
    target_language: Option<crate::language::LanguageId>,
}

impl LivePrefixReplacementAction {
    pub fn delete_previous_chars(&self) -> usize {
        self.delete_previous_chars
    }

    pub fn replacement(&self) -> &ReplacementText {
        &self.replacement
    }

    pub fn original_visible(&self) -> &str {
        &self.original_visible
    }

    pub fn target_language(&self) -> Option<&crate::language::LanguageId> {
        self.target_language.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoReplacementAction {
    Completed(ReplacementAction),
    LivePrefix(LivePrefixReplacementAction),
}

impl UndoReplacementAction {
    pub fn replacement(&self) -> &ReplacementText {
        match self {
            Self::Completed(action) => action.replacement(),
            Self::LivePrefix(action) => action.replacement(),
        }
    }

    pub fn boundary(&self) -> Boundary {
        match self {
            Self::Completed(action) => action.boundary(),
            Self::LivePrefix(_) => Boundary::Character('\0'),
        }
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
        let (replacement, target_language) = replacement.into_parts();

        Some(ReplacementAction {
            delete_previous_chars: token.text().chars().count(),
            replacement,
            boundary: token.boundary(),
            target_language,
        })
    }

    pub fn plan_live_prefix(
        &self,
        delete_previous_chars: usize,
        original_visible: &str,
        replacement: &str,
        target_language: Option<crate::language::LanguageId>,
    ) -> Option<LivePrefixReplacementAction> {
        Some(LivePrefixReplacementAction {
            delete_previous_chars,
            replacement: ReplacementText::try_new(replacement).ok()?,
            original_visible: original_visible.to_owned(),
            target_language,
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
            target_language: None,
        })
    }

    pub fn plan_live_prefix_undo(
        &self,
        original_text: &str,
        applied: &LivePrefixReplacementAction,
    ) -> Option<LivePrefixReplacementAction> {
        self.plan_live_prefix(
            applied.replacement().as_str().chars().count(),
            applied.replacement().as_str(),
            original_text,
            None,
        )
    }
}
