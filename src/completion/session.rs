use std::collections::HashSet;

use crate::input::{Boundary, InputBuffer, InputEvent, InputOutcome};
use crate::language::{TextCasePattern, normalize_word};

use super::CompletionProvider;
use super::sequence::MAX_CONTEXT_WORDS;

pub const DEFAULT_COMPLETION_PREFIX_CHARS: usize = 3;
pub const DEFAULT_COMPLETION_LIMIT: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionCommand {
    Previous,
    Next,
    Accept,
    AcceptNextWord,
    DeleteSelected,
    Dismiss,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CompletionDeletionTarget {
    TextHistory(String),
    Word(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionCommandResult {
    Pass,
    Consumed,
    AcceptSuffix(String),
    AcceptWord(String),
    DeletePrediction(CompletionDeletionTarget),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionApplyOutcome {
    Applied,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionSuggestion {
    text: String,
    suffix: String,
    deletion_target: Option<CompletionDeletionTarget>,
}

impl CompletionSuggestion {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn suffix(&self) -> &str {
        &self.suffix
    }

    pub fn deletion_target(&self) -> Option<&CompletionDeletionTarget> {
        self.deletion_target.as_ref()
    }
}

#[derive(Debug)]
struct PendingWordAcceptance {
    insertion: String,
    protected_continuation: Option<String>,
}

#[derive(Debug)]
pub struct CompletionSession {
    input: InputBuffer,
    context: Vec<String>,
    suggestions: Vec<CompletionSuggestion>,
    selected: usize,
    minimum_prefix_chars: usize,
    limit: usize,
    continuation_mode: bool,
    protected_continuation: Option<String>,
    pending_word_acceptance: Option<PendingWordAcceptance>,
    deleted_predictions: HashSet<CompletionDeletionTarget>,
}

impl Default for CompletionSession {
    fn default() -> Self {
        Self::new(DEFAULT_COMPLETION_PREFIX_CHARS, DEFAULT_COMPLETION_LIMIT)
    }
}

impl CompletionSession {
    pub fn new(minimum_prefix_chars: usize, limit: usize) -> Self {
        Self {
            input: InputBuffer::new(),
            context: Vec::new(),
            suggestions: Vec::new(),
            selected: 0,
            minimum_prefix_chars: minimum_prefix_chars.max(1),
            limit: limit.max(1),
            continuation_mode: false,
            protected_continuation: None,
            pending_word_acceptance: None,
            deleted_predictions: HashSet::new(),
        }
    }

    pub fn process_event(&mut self, event: InputEvent) {
        match self.input.process(event) {
            InputOutcome::Continue => {}
            InputOutcome::Completed(token) => {
                self.suggestions.clear();
                self.selected = 0;
                self.continuation_mode = false;
                self.protected_continuation = None;
                self.pending_word_acceptance = None;
                self.push_context(token.text());
                if !matches!(token.boundary(), Boundary::Character(' ')) {
                    self.context.clear();
                }
            }
            InputOutcome::Invalidated => self.dismiss_and_clear_context(),
        }

        if matches!(event, InputEvent::Backspace)
            && self.input.current_token().chars().count() < self.minimum_prefix_chars
            && !self.continuation_mode
        {
            self.dismiss();
        }
    }

    pub fn canonicalize_last_context_word(&mut self, canonical: &str) {
        if canonical.is_empty() {
            self.context.clear();
            return;
        }
        if let Some(last) = self.context.last_mut() {
            *last = canonical.to_owned();
        }
    }

    pub fn replace_current_prefix_for_live_layout(&mut self, replacement: &str) {
        self.pending_word_acceptance = None;
        self.protected_continuation = None;
        self.continuation_mode = false;
        self.input
            .replace_current_token_for_live_prefix(replacement);
    }

    pub fn refresh(&mut self, provider: &CompletionProvider, now_ms: i64) {
        let prefix = self.input.current_token();
        let prefix_chars = prefix.chars().count();
        let context_refs = self.context.iter().map(String::as_str).collect::<Vec<_>>();
        let has_context = !context_refs.is_empty();
        if prefix_chars < self.minimum_prefix_chars && !self.continuation_mode && !has_context {
            self.dismiss();
            return;
        }

        let normalized_prefix = normalize_word(prefix);
        if normalized_prefix.is_empty() && !self.continuation_mode && !has_context {
            self.dismiss();
            return;
        }

        let mut suggestions = Vec::new();
        let mut seen = HashSet::new();
        // While a previously visible continuation is protected, do not let the normal UI limit
        // hide a valid extension behind higher-ranked incompatible branches. SequenceHistory
        // already scans/sorts all matching rows before truncation, so this only defers truncation.
        let sequence_limit = if self.protected_continuation.is_some() {
            usize::MAX
        } else {
            self.limit
        };
        // Alt+Right keeps the already-visible next word as the logical lookup prefix while the
        // physical input prefix is empty. The provider therefore executes exactly the same
        // prefix-driven lookup/ranking path as it does for manual typing; only session lifecycle
        // decides where that prefix comes from.
        let lookup_prefix = if prefix.is_empty() {
            self.protected_continuation
                .as_deref()
                .and_then(|continuation| continuation.split_whitespace().next())
                .unwrap_or(prefix)
        } else {
            prefix
        };
        let sequence_candidates = provider.complete_sequence_for_prefix(
            &context_refs,
            lookup_prefix,
            now_ms,
            sequence_limit,
        );
        for candidate in sequence_candidates {
            let target = CompletionDeletionTarget::TextHistory(candidate.source_text().to_owned());
            if self.deleted_predictions.contains(&target) {
                continue;
            }
            if let Some(suggestion) =
                suggestion_for_prefix_with_target(candidate.text(), prefix, Some(target))
                && seen.insert(normalize_word(suggestion.text()))
            {
                suggestions.push(suggestion);
            }
        }

        if prefix_chars >= self.minimum_prefix_chars && suggestions.len() < self.limit {
            for candidate in provider.complete(prefix, self.limit) {
                let target = Some(CompletionDeletionTarget::Word(normalize_word(
                    candidate.text(),
                )));
                if target
                    .as_ref()
                    .is_some_and(|target| self.deleted_predictions.contains(target))
                {
                    continue;
                }
                if let Some(suggestion) =
                    suggestion_for_prefix_with_target(candidate.text(), prefix, target)
                    && seen.insert(normalize_word(suggestion.text()))
                {
                    suggestions.push(suggestion);
                    if suggestions.len() == self.limit {
                        break;
                    }
                }
            }
        }

        if let Some(protected) = self.protected_continuation.clone() {
            if suggestion_for_prefix(&protected, prefix).is_some() {
                let extending_index = suggestions.iter().position(|suggestion| {
                    continuation_strictly_extends_prefix(suggestion.text(), &protected)
                });
                let preserving_index = suggestions.iter().position(|suggestion| {
                    continuation_preserves_prefix(suggestion.text(), &protected)
                });
                if let Some(index) = extending_index.or(preserving_index) {
                    let current = suggestions.remove(index);
                    suggestions.insert(0, current);
                } else if let Some(current) = suggestion_for_prefix(&protected, prefix) {
                    let normalized_current = normalize_word(current.text());
                    suggestions.retain(|suggestion| {
                        normalize_word(suggestion.text()) != normalized_current
                    });
                    suggestions.insert(0, current);
                    suggestions.truncate(self.limit);
                }

                suggestions.truncate(self.limit);
                self.suggestions = suggestions;
                if self.suggestions.is_empty() {
                    self.dismiss();
                } else {
                    self.selected = 0;
                }
                return;
            }
            self.protected_continuation = None;
        }

        self.suggestions = suggestions;
        if self.suggestions.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.suggestions.len() - 1);
        }
    }

    pub fn command(&mut self, command: CompletionCommand) -> CompletionCommandResult {
        if self.suggestions.is_empty() {
            return CompletionCommandResult::Pass;
        }

        match command {
            CompletionCommand::Previous => {
                self.protected_continuation = None;
                self.selected = if self.selected == 0 {
                    self.suggestions.len() - 1
                } else {
                    self.selected - 1
                };
                CompletionCommandResult::Consumed
            }
            CompletionCommand::Next => {
                self.protected_continuation = None;
                self.selected = (self.selected + 1) % self.suggestions.len();
                CompletionCommandResult::Consumed
            }
            CompletionCommand::Accept => {
                let suffix = self.suggestions[self.selected].suffix.clone();
                self.dismiss();
                if suffix.is_empty() {
                    CompletionCommandResult::Consumed
                } else {
                    CompletionCommandResult::AcceptSuffix(suffix)
                }
            }
            CompletionCommand::AcceptNextWord => {
                let suggestion = &self.suggestions[self.selected];
                let Some((insertion, protected_continuation)) =
                    next_word_acceptance(suggestion.text(), self.input.current_token())
                else {
                    return CompletionCommandResult::Consumed;
                };
                self.pending_word_acceptance = Some(PendingWordAcceptance {
                    insertion: insertion.clone(),
                    protected_continuation,
                });
                CompletionCommandResult::AcceptWord(insertion)
            }
            CompletionCommand::DeleteSelected => {
                self.protected_continuation = None;
                self.pending_word_acceptance = None;
                let removed = self.suggestions.remove(self.selected);
                if self.suggestions.is_empty() {
                    self.selected = 0;
                } else {
                    self.selected = self.selected.min(self.suggestions.len() - 1);
                }
                if let Some(target) = removed.deletion_target {
                    self.deleted_predictions.insert(target.clone());
                    CompletionCommandResult::DeletePrediction(target)
                } else {
                    CompletionCommandResult::Consumed
                }
            }
            CompletionCommand::Dismiss => {
                self.dismiss();
                CompletionCommandResult::Consumed
            }
        }
    }

    pub fn word_acceptance_outcome(&mut self, outcome: CompletionApplyOutcome) {
        let Some(pending) = self.pending_word_acceptance.take() else {
            return;
        };
        if outcome != CompletionApplyOutcome::Applied {
            self.dismiss_and_clear_context();
            return;
        }

        self.continuation_mode = false;
        self.protected_continuation = None;
        for character in pending.insertion.chars() {
            let event = if character == ' ' {
                InputEvent::Boundary(Boundary::Character(' '))
            } else {
                InputEvent::character(character)
            };
            self.process_event(event);
        }
        self.continuation_mode = true;
        self.protected_continuation = pending.protected_continuation;
    }

    pub fn suggestions(&self) -> &[CompletionSuggestion] {
        &self.suggestions
    }

    pub const fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn current_prefix(&self) -> &str {
        self.input.current_token()
    }

    pub fn context_tokens(&self) -> &[String] {
        &self.context
    }

    pub fn is_active(&self) -> bool {
        !self.suggestions.is_empty()
    }

    pub fn dismiss(&mut self) {
        self.suggestions.clear();
        self.selected = 0;
        self.continuation_mode = false;
        self.protected_continuation = None;
        self.pending_word_acceptance = None;
    }

    pub fn dismiss_and_clear_context(&mut self) {
        self.dismiss();
        self.context.clear();
    }

    fn push_context(&mut self, token: &str) {
        if token.is_empty() {
            return;
        }
        self.context.push(token.to_owned());
        if self.context.len() > MAX_CONTEXT_WORDS {
            let excess = self.context.len() - MAX_CONTEXT_WORDS;
            self.context.drain(..excess);
        }
    }
}

fn next_word_acceptance(text: &str, prefix: &str) -> Option<(String, Option<String>)> {
    let (first_word, tail) = text
        .split_once(' ')
        .map_or((text, None), |(first, tail)| (first, Some(tail)));
    let normalized_prefix = normalize_word(prefix);
    if !normalize_word(first_word).starts_with(&normalized_prefix) {
        return None;
    }

    let prefix_chars = prefix.chars().count();
    if prefix_chars > first_word.chars().count() {
        return None;
    }
    let mut insertion = first_word.chars().skip(prefix_chars).collect::<String>();
    insertion.push(' ');
    let protected_continuation = tail.filter(|tail| !tail.is_empty()).map(str::to_owned);
    Some((insertion, protected_continuation))
}

fn continuation_tokens(text: &str) -> Vec<String> {
    text.split_whitespace().map(normalize_word).collect()
}

fn continuation_preserves_prefix(candidate: &str, protected: &str) -> bool {
    let candidate_tokens = continuation_tokens(candidate);
    let protected_tokens = continuation_tokens(protected);
    candidate_tokens.len() >= protected_tokens.len()
        && candidate_tokens[..protected_tokens.len()] == protected_tokens
}

fn continuation_strictly_extends_prefix(candidate: &str, protected: &str) -> bool {
    let candidate_tokens = continuation_tokens(candidate);
    let protected_tokens = continuation_tokens(protected);
    candidate_tokens.len() > protected_tokens.len()
        && candidate_tokens[..protected_tokens.len()] == protected_tokens
}

fn suggestion_for_prefix(text: &str, prefix: &str) -> Option<CompletionSuggestion> {
    suggestion_for_prefix_with_target(text, prefix, None)
}

fn suggestion_for_prefix_with_target(
    text: &str,
    prefix: &str,
    deletion_target: Option<CompletionDeletionTarget>,
) -> Option<CompletionSuggestion> {
    let prefix_chars = prefix.chars().count();
    let text_chars = text.chars().count();
    if prefix_chars > text_chars || !normalize_word(text).starts_with(&normalize_word(prefix)) {
        return None;
    }

    let display_text = completion_text_preserving_prefix_case(text, prefix)?;
    let suffix = display_text.chars().skip(prefix_chars).collect::<String>();
    Some(CompletionSuggestion {
        text: display_text,
        suffix,
        deletion_target,
    })
}

fn completion_text_preserving_prefix_case(text: &str, prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        return Some(text.to_owned());
    }

    let first_word_end = text
        .char_indices()
        .find(|(_, character)| character.is_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    let (first_word, tail) = text.split_at(first_word_end);
    let prefix_chars = prefix.chars().count();
    if prefix_chars > first_word.chars().count() {
        return None;
    }

    let cased_word = if TextCasePattern::detect(first_word) == TextCasePattern::Mixed {
        first_word.to_owned()
    } else {
        TextCasePattern::detect(prefix).apply(first_word)
    };
    let missing = cased_word.chars().skip(prefix_chars).collect::<String>();
    Some(format!("{prefix}{missing}{tail}"))
}

#[cfg(test)]
mod tests {
    use super::{
        CompletionCommand, CompletionCommandResult, CompletionSession, next_word_acceptance,
        suggestion_for_prefix,
    };
    use crate::input::{Boundary, InputEvent};

    #[test]
    fn suggestion_suffix_contains_only_missing_text() {
        let suggestion = suggestion_for_prefix("сделать проект", "сде").unwrap();
        assert_eq!(suggestion.text(), "сделать проект");
        assert_eq!(suggestion.suffix(), "лать проект");
    }

    #[test]
    fn suggestion_preserves_the_users_typed_case_instead_of_stored_case() {
        let lower = suggestion_for_prefix("Сделать проект", "сде").unwrap();
        assert_eq!(lower.text(), "сделать проект");
        assert_eq!(lower.suffix(), "лать проект");

        let upper = suggestion_for_prefix("сделать проект", "СДЕ").unwrap();
        assert_eq!(upper.text(), "СДЕЛАТЬ проект");
        assert_eq!(upper.suffix(), "ЛАТЬ проект");

        let mixed = suggestion_for_prefix("Сделать проект", "сДе").unwrap();
        assert_eq!(mixed.text(), "сДелать проект");
        assert_eq!(mixed.suffix(), "лать проект");

        let identifier = suggestion_for_prefix("PrototypeThing", "pro").unwrap();
        assert_eq!(identifier.text(), "prototypeThing");
        assert_eq!(identifier.suffix(), "totypeThing");
    }

    #[test]
    fn next_word_acceptance_advances_one_word_and_keeps_the_tail() {
        let (insertion, tail) = next_word_acceptance("сделать проект дальше", "сде").unwrap();
        assert_eq!(insertion, "лать ");
        assert_eq!(tail.as_deref(), Some("проект дальше"));
    }

    #[test]
    fn invalidation_clears_prefix_context_and_popup_state() {
        let mut session = CompletionSession::default();
        for character in "abc".chars() {
            session.process_event(InputEvent::character(character));
        }
        session.process_event(InputEvent::Invalidate);
        assert_eq!(session.current_prefix(), "");
        assert!(!session.is_active());
    }

    #[test]
    fn canonical_context_word_replaces_raw_token_and_empty_clears_context() {
        let mut session = CompletionSession::default();
        session.context.push("hello,".to_owned());
        session.canonicalize_last_context_word("hello");
        assert_eq!(session.context, ["hello"]);

        session.canonicalize_last_context_word("");
        assert!(session.context.is_empty());
    }

    #[test]
    fn non_space_boundary_ends_sequence_context() {
        let mut session = CompletionSession::default();
        for character in "hello".chars() {
            session.process_event(InputEvent::character(character));
        }
        session.process_event(InputEvent::Boundary(Boundary::Enter));
        for character in "wor".chars() {
            session.process_event(InputEvent::character(character));
        }
        assert_eq!(session.current_prefix(), "wor");
    }

    #[test]
    fn inactive_commands_pass_through() {
        let mut session = CompletionSession::default();
        assert_eq!(
            session.command(CompletionCommand::Dismiss),
            CompletionCommandResult::Pass
        );
    }
}
