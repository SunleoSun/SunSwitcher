use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

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
    text: Box<str>,
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
            text: text.into_boxed_str(),
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

type TokenId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ContextKey {
    len: u8,
    token_ids: [TokenId; MAX_CONTEXT_WORDS],
}

impl ContextKey {
    fn from_slice(token_ids: &[TokenId]) -> Self {
        debug_assert!(token_ids.len() <= MAX_CONTEXT_WORDS);
        let mut key = Self {
            len: token_ids.len() as u8,
            token_ids: [0; MAX_CONTEXT_WORDS],
        };
        key.token_ids[..token_ids.len()].copy_from_slice(token_ids);
        key
    }
}

#[derive(Debug, Clone)]
enum SequencePostings {
    One(usize),
    Many(Vec<usize>),
}

impl SequencePostings {
    fn push(&mut self, entry_index: usize) {
        match self {
            Self::One(existing) => *self = Self::Many(vec![*existing, entry_index]),
            Self::Many(entries) => entries.push(entry_index),
        }
    }

    fn for_each(&self, mut visit: impl FnMut(usize)) {
        match self {
            Self::One(entry_index) => visit(*entry_index),
            Self::Many(entries) => entries.iter().copied().for_each(visit),
        }
    }
}

fn push_posting<K: Eq + std::hash::Hash>(
    index: &mut HashMap<K, SequencePostings>,
    key: K,
    entry_index: usize,
) {
    match index.entry(key) {
        Entry::Vacant(slot) => {
            slot.insert(SequencePostings::One(entry_index));
        }
        Entry::Occupied(mut slot) => slot.get_mut().push(entry_index),
    }
}

#[derive(Debug, Clone)]
struct IndexedSequence {
    normalized_tokens: Box<[TokenId]>,
}

#[derive(Debug, Clone, Default)]
pub struct SequenceHistory {
    entries: Vec<SequenceCandidate>,
    indexed: Vec<IndexedSequence>,
    token_ids: BTreeMap<Arc<str>, TokenId>,
    tokens: Vec<Arc<str>>,
    next_token_index: HashMap<TokenId, SequencePostings>,
    context_index: HashMap<ContextKey, SequencePostings>,
}

impl SequenceHistory {
    pub fn new(entries: Vec<SequenceCandidate>) -> Self {
        let mut token_ids = BTreeMap::<Arc<str>, TokenId>::new();
        let mut tokens = Vec::<Arc<str>>::new();
        let indexed = entries
            .iter()
            .map(|entry| {
                let normalized_tokens = entry
                    .text
                    .split_whitespace()
                    .map(|token| {
                        let normalized = normalize_word(token);
                        if let Some(&id) = token_ids.get(normalized.as_str()) {
                            id
                        } else {
                            let id = TokenId::try_from(tokens.len())
                                .expect("sequence token count exceeds u32 capacity");
                            let normalized: Arc<str> = Arc::from(normalized);
                            tokens.push(Arc::clone(&normalized));
                            token_ids.insert(normalized, id);
                            id
                        }
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice();
                IndexedSequence { normalized_tokens }
            })
            .collect::<Vec<_>>();

        let mut next_token_index = HashMap::<TokenId, SequencePostings>::new();
        let mut context_index = HashMap::<ContextKey, SequencePostings>::new();
        for (entry_index, sequence) in indexed.iter().enumerate() {
            let max_specificity =
                MAX_CONTEXT_WORDS.min(sequence.normalized_tokens.len().saturating_sub(1));
            for specificity in 0..=max_specificity {
                let Some(&next_word) = sequence.normalized_tokens.get(specificity) else {
                    continue;
                };
                push_posting(&mut next_token_index, next_word, entry_index);
                if specificity > 0 {
                    push_posting(
                        &mut context_index,
                        ContextKey::from_slice(&sequence.normalized_tokens[..specificity]),
                        entry_index,
                    );
                }
            }
        }

        Self {
            entries,
            indexed,
            token_ids,
            tokens,
            next_token_index,
            context_index,
        }
    }

    pub fn entries(&self) -> &[SequenceCandidate] {
        &self.entries
    }

    fn candidate_indices_for_lookup(
        &self,
        normalized_context: &[String],
        normalized_prefix: &str,
    ) -> Vec<usize> {
        let mut seen = HashSet::new();
        let mut candidates = Vec::new();
        if !normalized_prefix.is_empty() {
            for (token, &token_id) in self.token_ids.range(Arc::<str>::from(normalized_prefix)..) {
                if !token.starts_with(normalized_prefix) {
                    break;
                }
                if let Some(postings) = self.next_token_index.get(&token_id) {
                    postings.for_each(|entry_index| {
                        if seen.insert(entry_index) {
                            candidates.push(entry_index);
                        }
                    });
                }
            }
            return candidates;
        }

        let max_context = normalized_context.len().min(MAX_CONTEXT_WORDS);
        for specificity in (1..=max_context).rev() {
            let context_start = normalized_context.len() - specificity;
            let mut context_ids = [0; MAX_CONTEXT_WORDS];
            let mut known_count = 0;
            for token in &normalized_context[context_start..] {
                let Some(&token_id) = self.token_ids.get(token.as_str()) else {
                    break;
                };
                context_ids[known_count] = token_id;
                known_count += 1;
            }
            if known_count != specificity {
                continue;
            }
            let key = ContextKey::from_slice(&context_ids[..specificity]);
            if let Some(postings) = self.context_index.get(&key) {
                postings.for_each(|entry_index| {
                    if seen.insert(entry_index) {
                        candidates.push(entry_index);
                    }
                });
            }
        }
        candidates
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
        let normalized_context_ids = normalized_context
            .iter()
            .map(|token| self.token_ids.get(token.as_str()).copied())
            .collect::<Vec<_>>();
        let normalized_prefix = normalize_word(prefix);
        if normalized_context.is_empty() && normalized_prefix.is_empty() {
            return Vec::new();
        }

        let candidate_indices =
            self.candidate_indices_for_lookup(&normalized_context, &normalized_prefix);
        let mut completions = Vec::new();
        for entry_index in candidate_indices {
            let entry = &self.entries[entry_index];
            let normalized_sequence = &self.indexed[entry_index].normalized_tokens;
            if normalized_sequence.is_empty() {
                continue;
            }

            let max_specificity = normalized_context
                .len()
                .min(MAX_CONTEXT_WORDS)
                .min(normalized_sequence.len().saturating_sub(1));
            let specificity_matches = |specificity: usize| {
                let context_matches = if specificity == 0 {
                    true
                } else {
                    let context_start = normalized_context_ids.len() - specificity;
                    normalized_context_ids[context_start..]
                        .iter()
                        .zip(&normalized_sequence[..specificity])
                        .all(|(context_id, sequence_id)| context_id == &Some(*sequence_id))
                };
                context_matches
                    && normalized_sequence
                        .get(specificity)
                        .and_then(|&token_id| self.tokens.get(token_id as usize))
                        .is_some_and(|next_word| next_word.starts_with(&normalized_prefix))
            };

            let matched_context_words = if normalized_prefix.is_empty() {
                (0..=max_specificity)
                    .rev()
                    .find(|&specificity| specificity_matches(specificity))
            } else {
                (0..=max_specificity)
                    .rev()
                    .find(|&specificity| {
                        specificity_matches(specificity)
                            && normalized_sequence.len().saturating_sub(specificity) >= 2
                    })
                    .or_else(|| {
                        (0..=max_specificity)
                            .rev()
                            .find(|&specificity| specificity_matches(specificity))
                    })
            };
            let Some(matched_context_words) = matched_context_words else {
                continue;
            };
            if normalized_prefix.is_empty()
                && !normalized_context.is_empty()
                && matched_context_words == 0
            {
                continue;
            }

            completions.push(SequenceCompletion {
                text: entry
                    .text()
                    .split_whitespace()
                    .skip(matched_context_words)
                    .collect::<Vec<_>>()
                    .join(" "),
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
    fn repeated_prefix_keeps_a_multi_word_continuation_when_context_also_matches() {
        let history = SequenceHistory::new(vec![
            SequenceCandidate::try_new("AA11 AA33", 1, DAY_MS).unwrap(),
            SequenceCandidate::try_new("AA33 AA33", 3, DAY_MS).unwrap(),
            SequenceCandidate::try_new("AA33 AA33 AA33", 2, DAY_MS).unwrap(),
        ]);

        let completions = history.complete_for_prefix(&["AA33", "AA33"], "AA3", DAY_MS, 10);
        assert_eq!(completions[0].text(), "AA33 AA33");
    }

    #[test]
    fn one_lookup_rule_uses_the_visible_prefix_to_preserve_repeated_continuations() {
        let history = SequenceHistory::new(vec![
            SequenceCandidate::try_new("AA33 AA33 AA33", 3, DAY_MS).unwrap(),
        ]);

        let typed = history.complete_for_prefix(&["AA33", "AA33"], "AA3", DAY_MS, 10);
        assert_eq!(typed[0].text(), "AA33 AA33");
        assert_eq!(typed[0].matched_context_words(), 1);

        let no_prefix = history.complete_for_prefix(&["AA33", "AA33"], "", DAY_MS, 10);
        assert_eq!(no_prefix[0].text(), "AA33");
        assert_eq!(no_prefix[0].matched_context_words(), 2);
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

    #[test]
    fn lookup_index_prunes_unrelated_sequence_history() {
        let mut entries = (0..2_000)
            .map(|index| {
                SequenceCandidate::try_new(format!("noise{index} tail"), 1, DAY_MS).unwrap()
            })
            .collect::<Vec<_>>();
        let target_index = entries.len();
        entries.push(SequenceCandidate::try_new("project alpha beta", 1, DAY_MS).unwrap());
        let history = SequenceHistory::new(entries);

        assert_eq!(
            history.candidate_indices_for_lookup(&[], "pro"),
            vec![target_index]
        );
        assert_eq!(
            history.candidate_indices_for_lookup(&["project".to_owned()], ""),
            vec![target_index]
        );
    }
}
