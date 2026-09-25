use std::cmp::Ordering;
use std::collections::HashSet;

use crate::language::normalize_word;

pub const MAX_CONTEXT_WORDS: usize = 4;
pub const MAX_SEQUENCE_WORDS: usize = MAX_CONTEXT_WORDS + 1;
const DAY_MS: f64 = 86_400_000.0;
const RECENCY_DECAY_DAYS: f64 = 14.0;
const REPETITION_WEIGHT: f64 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceHistoryError {
    InvalidText,
    InvalidUseCount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceCandidate {
    text: String,
    use_count: u32,
    last_used_at_ms: i64,
}

impl SequenceCandidate {
    pub fn try_new(
        text: impl Into<String>,
        use_count: u32,
        last_used_at_ms: i64,
    ) -> Result<Self, SequenceHistoryError> {
        let text = text.into();
        if text.contains('\0') || text.split_whitespace().count() < 2 {
            return Err(SequenceHistoryError::InvalidText);
        }
        if use_count == 0 {
            return Err(SequenceHistoryError::InvalidUseCount);
        }
        Ok(Self {
            text,
            use_count,
            last_used_at_ms,
        })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub const fn use_count(&self) -> u32 {
        self.use_count
    }

    pub const fn last_used_at_ms(&self) -> i64 {
        self.last_used_at_ms
    }

    pub fn user_score(&self, now_ms: i64) -> f64 {
        let repeat_count = self.use_count.saturating_sub(1) as f64;
        let repetition = REPETITION_WEIGHT * (1.0 + repeat_count).ln();
        let age_ms = now_ms.saturating_sub(self.last_used_at_ms).max(0) as f64;
        let age_days = age_ms / DAY_MS;
        let recency = (-age_days / RECENCY_DECAY_DAYS).exp();
        repetition + recency
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SequenceCompletion {
    text: String,
    source_text: String,
    matched_context_words: usize,
    user_score: f64,
}

impl SequenceCompletion {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn source_text(&self) -> &str {
        &self.source_text
    }

    pub const fn matched_context_words(&self) -> usize {
        self.matched_context_words
    }

    pub const fn user_score(&self) -> f64 {
        self.user_score
    }
}

#[derive(Debug, Clone, Default)]
pub struct SequenceHistory {
    entries: Vec<SequenceCandidate>,
}

impl SequenceHistory {
    pub fn new(entries: Vec<SequenceCandidate>) -> Self {
        Self { entries }
    }

    pub fn entries(&self) -> &[SequenceCandidate] {
        &self.entries
    }

    pub fn complete(
        &self,
        context_tokens: &[&str],
        now_ms: i64,
        limit: usize,
    ) -> Vec<SequenceCompletion> {
        self.complete_for_prefix(context_tokens, "", now_ms, limit)
    }

    pub fn complete_for_prefix(
        &self,
        context_tokens: &[&str],
        prefix: &str,
        now_ms: i64,
        limit: usize,
    ) -> Vec<SequenceCompletion> {
        if limit == 0 {
            return Vec::new();
        }

        let normalized_context: Vec<_> = context_tokens
            .iter()
            .map(|token| normalize_word(token))
            .filter(|token| !token.is_empty())
            .collect();
        let normalized_prefix = normalize_word(prefix);
        if normalized_context.is_empty() && normalized_prefix.is_empty() {
            return Vec::new();
        }

        let mut completions = Vec::new();
        for entry in &self.entries {
            let sequence_tokens: Vec<_> = entry.text.split_whitespace().collect();
            let normalized_sequence: Vec<_> = sequence_tokens
                .iter()
                .map(|token| normalize_word(token))
                .collect();
            if normalized_sequence.is_empty() {
                continue;
            }

            let max_specificity = normalized_context
                .len()
                .min(MAX_CONTEXT_WORDS)
                .min(normalized_sequence.len().saturating_sub(1));
            let matched_context_words = if max_specificity == 0 {
                0
            } else {
                (1..=max_specificity)
                    .rev()
                    .find(|specificity| {
                        let context_start = normalized_context.len() - specificity;
                        normalized_context[context_start..] == normalized_sequence[..*specificity]
                    })
                    .unwrap_or(0)
            };

            if normalized_prefix.is_empty()
                && !normalized_context.is_empty()
                && matched_context_words == 0
            {
                continue;
            }
            let Some(next_word) = normalized_sequence.get(matched_context_words) else {
                continue;
            };
            if !next_word.starts_with(&normalized_prefix) {
                continue;
            }

            completions.push(SequenceCompletion {
                text: sequence_tokens[matched_context_words..].join(" "),
                source_text: entry.text().to_owned(),
                matched_context_words,
                user_score: entry.user_score(now_ms),
            });
        }

        completions.sort_by(|left, right| {
            right
                .matched_context_words
                .cmp(&left.matched_context_words)
                .then_with(|| {
                    right
                        .user_score
                        .partial_cmp(&left.user_score)
                        .unwrap_or(Ordering::Equal)
                })
                .then_with(|| {
                    right
                        .text
                        .split_whitespace()
                        .count()
                        .cmp(&left.text.split_whitespace().count())
                })
                .then_with(|| left.text.cmp(&right.text))
        });
        let mut seen = HashSet::new();
        completions.retain(|completion| seen.insert(normalize_word(&completion.text)));
        completions.truncate(limit);
        completions
    }

    pub fn ngrams_from_canonical_tokens(tokens: &[String]) -> Vec<String> {
        let mut sequences = Vec::new();
        for start in 0..tokens.len() {
            let max_len = (tokens.len() - start).min(MAX_SEQUENCE_WORDS);
            for len in 2..=max_len {
                sequences.push(tokens[start..start + len].join(" "));
            }
        }
        sequences
    }

    pub fn suffix_ngrams_from_canonical_tokens(tokens: &[String]) -> Vec<String> {
        let max_len = tokens.len().min(MAX_SEQUENCE_WORDS);
        (2..=max_len)
            .map(|len| tokens[tokens.len() - len..].join(" "))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{SequenceCandidate, SequenceHistory};

    const DAY_MS: i64 = 86_400_000;

    #[test]
    fn longer_context_is_ranked_before_higher_user_score() {
        let history = SequenceHistory::new(vec![
            SequenceCandidate::try_new("мне нужно сделать", 2, 10 * DAY_MS).unwrap(),
            SequenceCandidate::try_new("нужно проверить", 100, 10 * DAY_MS).unwrap(),
        ]);

        let completions = history.complete(&["мне", "нужно"], 10 * DAY_MS, 10);
        assert_eq!(completions[0].text(), "сделать");
        assert_eq!(completions[0].matched_context_words(), 2);
        assert_eq!(completions[1].text(), "проверить");
        assert_eq!(completions[1].matched_context_words(), 1);
    }

    #[test]
    fn repeated_sequence_beats_single_fresh_observation_for_same_context() {
        let history = SequenceHistory::new(vec![
            SequenceCandidate::try_new("hello world", 5, 0).unwrap(),
            SequenceCandidate::try_new("hello there", 1, 100 * DAY_MS).unwrap(),
        ]);

        let completions = history.complete(&["hello"], 100 * DAY_MS, 10);
        assert_eq!(completions[0].text(), "world");
        assert_eq!(completions[1].text(), "there");
    }

    #[test]
    fn prefix_can_start_a_multi_word_completion_without_context() {
        let history = SequenceHistory::new(vec![
            SequenceCandidate::try_new("project alpha", 1, DAY_MS).unwrap(),
            SequenceCandidate::try_new("project alpha beta", 1, DAY_MS).unwrap(),
            SequenceCandidate::try_new("apple unrelated", 100, DAY_MS).unwrap(),
        ]);

        let completions = history.complete_for_prefix(&[], "pro", DAY_MS, 1);
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].text(), "project alpha beta");
        assert_eq!(completions[0].matched_context_words(), 0);
    }

    #[test]
    fn equal_evidence_prefers_the_longer_continuation() {
        let history = SequenceHistory::new(vec![
            SequenceCandidate::try_new("hello world", 1, DAY_MS).unwrap(),
            SequenceCandidate::try_new("hello world again", 1, DAY_MS).unwrap(),
        ]);

        let completions = history.complete(&["hello"], DAY_MS, 10);
        assert_eq!(completions[0].text(), "world again");
        assert_eq!(completions[1].text(), "world");
    }

    #[test]
    fn typed_suffix_ngrams_only_add_sequences_ending_at_the_newest_word() {
        let tokens = ["one", "two", "three", "four", "five", "six"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let ngrams = SequenceHistory::suffix_ngrams_from_canonical_tokens(&tokens);

        assert!(ngrams.contains(&"five six".to_owned()));
        assert!(ngrams.contains(&"two three four five six".to_owned()));
        assert!(!ngrams.contains(&"one two".to_owned()));
        assert_eq!(ngrams.len(), 4);
    }

    #[test]
    fn canonical_tokens_produce_bounded_contiguous_ngrams() {
        let tokens = ["one", "two", "three", "four", "five", "six"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let ngrams = SequenceHistory::ngrams_from_canonical_tokens(&tokens);

        assert!(ngrams.contains(&"one two".to_owned()));
        assert!(ngrams.contains(&"one two three four five".to_owned()));
        assert!(!ngrams.contains(&"one two three four five six".to_owned()));
        assert!(ngrams.contains(&"two three four five six".to_owned()));
    }
}
