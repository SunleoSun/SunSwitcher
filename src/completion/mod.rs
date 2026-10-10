pub mod sequence;
mod session;

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, RwLock};

use crate::correction::LexicalSnapshot;
use crate::language::normalize_word;
use crate::lexicon::UserWord;

use self::sequence::{SequenceCandidate, SequenceCompletion, SequenceHistory};

pub use session::{
    CompletionApplyOutcome, CompletionCommand, CompletionCommandResult, CompletionDeletionTarget,
    CompletionSession, CompletionSuggestion, DEFAULT_COMPLETION_LIMIT,
    DEFAULT_COMPLETION_PREFIX_CHARS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionSource {
    UserLexicon,
    SystemDictionary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionCandidate {
    text: String,
    source: CompletionSource,
}

impl CompletionCandidate {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub const fn source(&self) -> CompletionSource {
        self.source
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PrefixEvidence {
    pub has_exact_word: bool,
    pub best_user_use_count: u32,
    pub best_system_frequency: u32,
    pub best_sequence_score: f64,
    pub candidate_count_capped: u8,
}

impl PrefixEvidence {
    pub fn score(self) -> f32 {
        let exact = if self.has_exact_word { 2.0 } else { 0.0 };
        let user = if self.best_user_use_count == 0 {
            0.0
        } else {
            1.5 + (1.0 + self.best_user_use_count as f32).ln()
        };
        let system = if self.best_system_frequency == 0 {
            0.0
        } else {
            (1.0 + self.best_system_frequency as f32).ln() * 0.35
        };
        let sequence = (self.best_sequence_score as f32).min(4.0) * 0.8;
        let count = self.candidate_count_capped as f32 * 0.15;
        exact + user + system + sequence + count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionSnapshotError {
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct SequenceSnapshot {
    base: Arc<SequenceHistory>,
    overlay_entries: Arc<BTreeMap<String, SequenceCandidate>>,
    overlay: Arc<SequenceHistory>,
}

impl SequenceSnapshot {
    fn from_history(history: Arc<SequenceHistory>) -> Self {
        Self {
            base: history,
            overlay_entries: Arc::new(BTreeMap::new()),
            overlay: Arc::new(SequenceHistory::default()),
        }
    }

    fn with_updates(&self, updates: Vec<SequenceCandidate>) -> Self {
        let mut entries = (*self.overlay_entries).clone();
        for update in updates {
            entries.insert(update.text().to_owned(), update);
        }
        let overlay = SequenceHistory::new(entries.values().cloned().collect());
        Self {
            base: Arc::clone(&self.base),
            overlay_entries: Arc::new(entries),
            overlay: Arc::new(overlay),
        }
    }

    fn pending_len(&self) -> usize {
        self.overlay_entries.len()
    }

    fn complete(&self, context: &[&str], now_ms: i64, limit: usize) -> Vec<SequenceCompletion> {
        self.complete_for_prefix(context, "", now_ms, limit)
    }

    fn complete_for_prefix(
        &self,
        context: &[&str],
        prefix: &str,
        now_ms: i64,
        limit: usize,
    ) -> Vec<SequenceCompletion> {
        if limit == 0 {
            return Vec::new();
        }
        let mut completions = self
            .base
            .complete_for_prefix(context, prefix, now_ms, usize::MAX)
            .into_iter()
            .filter(|candidate| !self.overlay_entries.contains_key(candidate.source_text()))
            .collect::<Vec<_>>();
        completions.extend(
            self.overlay
                .complete_for_prefix(context, prefix, now_ms, usize::MAX),
        );
        completions.sort_by(sequence_completion_rank);
        let mut seen = HashSet::new();
        completions.retain(|item| seen.insert(normalize_word(item.text())));
        completions.truncate(limit);
        completions
    }
}

impl From<Arc<SequenceHistory>> for SequenceSnapshot {
    fn from(history: Arc<SequenceHistory>) -> Self {
        Self::from_history(history)
    }
}

impl From<Arc<SequenceSnapshot>> for SequenceSnapshot {
    fn from(snapshot: Arc<SequenceSnapshot>) -> Self {
        (*snapshot).clone()
    }
}

fn sequence_completion_rank(left: &SequenceCompletion, right: &SequenceCompletion) -> Ordering {
    right
        .matched_context_words()
        .cmp(&left.matched_context_words())
        .then_with(|| {
            right
                .user_score()
                .partial_cmp(&left.user_score())
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| {
            right
                .text()
                .split_whitespace()
                .count()
                .cmp(&left.text().split_whitespace().count())
        })
        .then_with(|| left.text().cmp(right.text()))
}

#[derive(Debug, Clone)]
pub struct SequenceSnapshotStore {
    current: Arc<RwLock<Arc<SequenceSnapshot>>>,
}

impl SequenceSnapshotStore {
    pub const REBASE_LIMIT: usize = 1024;
    pub const ACTIVE_REPEATED_BASE_LIMIT: usize = 225_000;
    pub const ACTIVE_RECENT_SINGLETON_BASE_LIMIT: usize = 25_000;

    pub fn new(history: SequenceHistory) -> Self {
        Self {
            current: Arc::new(RwLock::new(Arc::new(SequenceSnapshot::from_history(
                Arc::new(history),
            )))),
        }
    }

    pub fn load(&self) -> Result<Arc<SequenceSnapshot>, CompletionSnapshotError> {
        self.current
            .read()
            .map(|snapshot| Arc::clone(&snapshot))
            .map_err(|_| CompletionSnapshotError::Unavailable)
    }

    pub fn apply_updates(
        &self,
        updates: Vec<SequenceCandidate>,
    ) -> Result<usize, CompletionSnapshotError> {
        if updates.is_empty() {
            return self.pending_len();
        }
        let replacement = Arc::new(self.load()?.with_updates(updates));
        let pending = replacement.pending_len();
        let mut slot = self
            .current
            .write()
            .map_err(|_| CompletionSnapshotError::Unavailable)?;
        *slot = replacement;
        Ok(pending)
    }

    pub fn pending_len(&self) -> Result<usize, CompletionSnapshotError> {
        self.load().map(|snapshot| snapshot.pending_len())
    }

    pub fn replace(&self, history: SequenceHistory) -> Result<(), CompletionSnapshotError> {
        let mut slot = self
            .current
            .write()
            .map_err(|_| CompletionSnapshotError::Unavailable)?;
        *slot = Arc::new(SequenceSnapshot::from_history(Arc::new(history)));
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct CompletionWordSuppressions {
    normalized_words: HashSet<String>,
}

impl CompletionWordSuppressions {
    pub fn from_normalized_words(words: impl IntoIterator<Item = String>) -> Self {
        Self {
            normalized_words: words
                .into_iter()
                .map(|word| normalize_word(&word))
                .filter(|word| !word.is_empty())
                .collect(),
        }
    }

    pub fn contains(&self, text: &str) -> bool {
        self.normalized_words.contains(&normalize_word(text))
    }

    pub fn len(&self) -> usize {
        self.normalized_words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.normalized_words.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct CompletionSuppressionSnapshotStore {
    current: Arc<RwLock<Arc<CompletionWordSuppressions>>>,
}

impl CompletionSuppressionSnapshotStore {
    pub fn new(suppressions: CompletionWordSuppressions) -> Self {
        Self {
            current: Arc::new(RwLock::new(Arc::new(suppressions))),
        }
    }

    pub fn load(&self) -> Result<Arc<CompletionWordSuppressions>, CompletionSnapshotError> {
        self.current
            .read()
            .map(|suppressions| Arc::clone(&suppressions))
            .map_err(|_| CompletionSnapshotError::Unavailable)
    }

    pub fn replace(
        &self,
        suppressions: CompletionWordSuppressions,
    ) -> Result<(), CompletionSnapshotError> {
        let mut slot = self
            .current
            .write()
            .map_err(|_| CompletionSnapshotError::Unavailable)?;
        *slot = Arc::new(suppressions);
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CompletionProvider {
    snapshot: Arc<LexicalSnapshot>,
    sequences: Arc<SequenceSnapshot>,
    word_suppressions: Arc<CompletionWordSuppressions>,
}

impl CompletionProvider {
    pub fn new<S>(snapshot: Arc<LexicalSnapshot>, sequences: S) -> Self
    where
        S: Into<SequenceSnapshot>,
    {
        Self::with_word_suppressions(
            snapshot,
            sequences,
            Arc::new(CompletionWordSuppressions::default()),
        )
    }

    pub fn with_word_suppressions<S>(
        snapshot: Arc<LexicalSnapshot>,
        sequences: S,
        word_suppressions: Arc<CompletionWordSuppressions>,
    ) -> Self
    where
        S: Into<SequenceSnapshot>,
    {
        Self {
            snapshot,
            sequences: Arc::new(sequences.into()),
            word_suppressions,
        }
    }

    pub fn lexical_snapshot(&self) -> &LexicalSnapshot {
        &self.snapshot
    }

    pub fn prefix_evidence(
        &self,
        context_tokens: &[&str],
        prefix: &str,
        now_ms: i64,
        limit: usize,
    ) -> PrefixEvidence {
        let normalized = normalize_completion_prefix(prefix);
        if normalized.is_empty() || limit == 0 {
            return PrefixEvidence::default();
        }

        let mut evidence = PrefixEvidence::default();
        if self.snapshot.contains_normalized(&normalized) {
            evidence.has_exact_word = true;
        }

        for word in self
            .snapshot
            .user_lexicon()
            .prefix_matches(
                &normalized,
                limit.saturating_add(self.snapshot.ignored_words().len()),
            )
            .into_iter()
            .filter(|word| {
                !self
                    .snapshot
                    .ignored_words()
                    .contains_normalized(word.normalized_term())
            })
            .take(limit)
        {
            evidence.best_user_use_count = evidence.best_user_use_count.max(word.use_count());
            evidence.candidate_count_capped = evidence.candidate_count_capped.saturating_add(1);
        }

        for language in self.snapshot.languages() {
            for entry in language
                .prefix_matches(
                    &normalized,
                    limit.saturating_add(self.snapshot.ignored_words().len()),
                )
                .into_iter()
                .filter(|entry| {
                    !self
                        .snapshot
                        .ignored_words()
                        .contains_normalized(entry.word())
                })
                .take(limit)
            {
                evidence.best_system_frequency =
                    evidence.best_system_frequency.max(entry.frequency());
                evidence.candidate_count_capped = evidence.candidate_count_capped.saturating_add(1);
            }
        }

        evidence.best_sequence_score = self
            .sequences
            .complete_for_prefix(context_tokens, &normalized, now_ms, limit)
            .into_iter()
            .filter(|candidate| self.sequence_completion_is_lexically_valid(candidate))
            .map(|candidate| candidate.user_score())
            .fold(0.0_f64, f64::max);
        evidence
    }

    pub fn complete(&self, prefix: &str, limit: usize) -> Vec<CompletionCandidate> {
        if limit == 0 {
            return Vec::new();
        }

        let lookup_limit = limit
            .saturating_add(self.word_suppressions.len())
            .saturating_add(self.snapshot.ignored_words().len());
        let mut ranked = self
            .snapshot
            .user_lexicon()
            .prefix_matches(prefix, lookup_limit)
            .into_iter()
            .filter(|word| {
                !self
                    .snapshot
                    .ignored_words()
                    .contains_normalized(word.normalized_term())
                    && !self.word_suppressions.contains(word.term())
            })
            .map(|word: &UserWord| RankedWordCandidate {
                candidate: CompletionCandidate {
                    text: word.term().to_owned(),
                    source: CompletionSource::UserLexicon,
                },
                source_priority: 1,
                recency: word.last_used_at_ms(),
                usage: word.use_count(),
            })
            .collect::<Vec<_>>();

        for language in self.snapshot.languages() {
            for entry in language.prefix_matches(prefix, lookup_limit) {
                if self
                    .snapshot
                    .ignored_words()
                    .contains_normalized(entry.word())
                    || self.word_suppressions.contains(entry.word())
                {
                    continue;
                }
                ranked.push(RankedWordCandidate {
                    candidate: CompletionCandidate {
                        text: entry.word().to_owned(),
                        source: CompletionSource::SystemDictionary,
                    },
                    source_priority: 0,
                    recency: 0,
                    usage: entry.frequency(),
                });
            }
        }

        ranked.sort_by(|left, right| {
            right
                .source_priority
                .cmp(&left.source_priority)
                .then_with(|| match (left.candidate.source, right.candidate.source) {
                    (CompletionSource::UserLexicon, CompletionSource::UserLexicon) => right
                        .recency
                        .cmp(&left.recency)
                        .then_with(|| right.usage.cmp(&left.usage)),
                    (CompletionSource::SystemDictionary, CompletionSource::SystemDictionary) => {
                        right.usage.cmp(&left.usage)
                    }
                    _ => Ordering::Equal,
                })
                .then_with(|| left.candidate.text.cmp(&right.candidate.text))
        });
        let mut seen = HashSet::new();
        ranked.retain(|ranked| seen.insert(normalize_word(&ranked.candidate.text)));
        ranked.truncate(limit);
        ranked.into_iter().map(|ranked| ranked.candidate).collect()
    }

    pub fn complete_sequence(
        &self,
        context_tokens: &[&str],
        now_ms: i64,
        limit: usize,
    ) -> Vec<SequenceCompletion> {
        if limit == 0 {
            return Vec::new();
        }
        self.sequences
            .complete(context_tokens, now_ms, usize::MAX)
            .into_iter()
            .filter(|candidate| self.sequence_completion_is_lexically_valid(candidate))
            .take(limit)
            .collect()
    }

    pub fn complete_sequence_for_prefix(
        &self,
        context_tokens: &[&str],
        prefix: &str,
        now_ms: i64,
        limit: usize,
    ) -> Vec<SequenceCompletion> {
        if limit == 0 {
            return Vec::new();
        }
        self.sequences
            .complete_for_prefix(context_tokens, prefix, now_ms, usize::MAX)
            .into_iter()
            .filter(|candidate| self.sequence_completion_is_lexically_valid(candidate))
            .take(limit)
            .collect()
    }

    fn sequence_completion_is_lexically_valid(&self, candidate: &SequenceCompletion) -> bool {
        candidate.text().split_whitespace().all(|token| {
            let normalized = normalize_word(token);
            !normalized.is_empty() && self.snapshot.contains_normalized(&normalized)
        })
    }
}

#[derive(Debug)]
struct RankedWordCandidate {
    candidate: CompletionCandidate,
    source_priority: u8,
    recency: i64,
    usage: u32,
}

fn normalize_completion_prefix(prefix: &str) -> String {
    let characters = prefix.chars().collect::<Vec<_>>();
    let mut start = 0;
    while start < characters.len() && !characters[start].is_alphanumeric() {
        start += 1;
    }
    let mut end = characters.len();
    while end > start && !characters[end - 1].is_alphanumeric() {
        end -= 1;
    }
    normalize_word(&characters[start..end].iter().collect::<String>())
}

#[cfg(test)]
mod snapshot_tests {
    use super::{SequenceCandidate, SequenceHistory, SequenceSnapshotStore};

    #[test]
    fn overlay_updates_shadow_stale_base_rows_and_reorder_immediately() {
        let alpha = SequenceCandidate::try_new("hello alpha", 1, 100).unwrap();
        let beta = SequenceCandidate::try_new("hello beta", 10, 100).unwrap();
        let store = SequenceSnapshotStore::new(SequenceHistory::new(vec![alpha, beta]));

        let updated_alpha = SequenceCandidate::try_new("hello alpha", 20, 200).unwrap();
        assert_eq!(store.apply_updates(vec![updated_alpha]).unwrap(), 1);

        let snapshot = store.load().unwrap();
        let completions = snapshot.complete_for_prefix(&[], "hel", 300, 10);
        assert_eq!(completions.len(), 2);
        assert_eq!(completions[0].source_text(), "hello alpha");
        assert_eq!(
            completions
                .iter()
                .filter(|item| item.source_text() == "hello alpha")
                .count(),
            1
        );
    }

    #[test]
    fn replacing_base_clears_overlay_after_rebase() {
        let base = SequenceCandidate::try_new("hello world", 1, 100).unwrap();
        let store = SequenceSnapshotStore::new(SequenceHistory::new(vec![base]));
        let update = SequenceCandidate::try_new("hello world", 2, 200).unwrap();
        store.apply_updates(vec![update.clone()]).unwrap();
        assert_eq!(store.pending_len().unwrap(), 1);

        store.replace(SequenceHistory::new(vec![update])).unwrap();
        assert_eq!(store.pending_len().unwrap(), 0);
    }
}
