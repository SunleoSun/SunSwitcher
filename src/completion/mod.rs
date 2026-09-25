pub mod sequence;
mod session;

use std::cmp::Ordering;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use crate::correction::LexicalSnapshot;
use crate::language::normalize_word;
use crate::lexicon::UserWord;

use self::sequence::{SequenceCompletion, SequenceHistory};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionSnapshotError {
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct SequenceSnapshotStore {
    current: Arc<RwLock<Arc<SequenceHistory>>>,
}

impl SequenceSnapshotStore {
    pub fn new(history: SequenceHistory) -> Self {
        Self {
            current: Arc::new(RwLock::new(Arc::new(history))),
        }
    }

    pub fn load(&self) -> Result<Arc<SequenceHistory>, CompletionSnapshotError> {
        self.current
            .read()
            .map(|history| Arc::clone(&history))
            .map_err(|_| CompletionSnapshotError::Unavailable)
    }

    pub fn replace(&self, history: SequenceHistory) -> Result<(), CompletionSnapshotError> {
        let mut slot = self
            .current
            .write()
            .map_err(|_| CompletionSnapshotError::Unavailable)?;
        *slot = Arc::new(history);
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
    sequences: Arc<SequenceHistory>,
    word_suppressions: Arc<CompletionWordSuppressions>,
}

impl CompletionProvider {
    pub fn new(snapshot: Arc<LexicalSnapshot>, sequences: Arc<SequenceHistory>) -> Self {
        Self::with_word_suppressions(
            snapshot,
            sequences,
            Arc::new(CompletionWordSuppressions::default()),
        )
    }

    pub fn with_word_suppressions(
        snapshot: Arc<LexicalSnapshot>,
        sequences: Arc<SequenceHistory>,
        word_suppressions: Arc<CompletionWordSuppressions>,
    ) -> Self {
        Self {
            snapshot,
            sequences,
            word_suppressions,
        }
    }

    pub fn complete(&self, prefix: &str, limit: usize) -> Vec<CompletionCandidate> {
        if limit == 0 {
            return Vec::new();
        }

        let lookup_limit = limit.saturating_add(self.word_suppressions.len());
        let mut ranked = self
            .snapshot
            .user_lexicon()
            .prefix_matches(prefix, lookup_limit)
            .into_iter()
            .filter(|word| !self.word_suppressions.contains(word.term()))
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
                if self.word_suppressions.contains(entry.word()) {
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
            !normalized.is_empty()
                && (self
                    .snapshot
                    .user_lexicon()
                    .contains_normalized(&normalized)
                    || self
                        .snapshot
                        .languages()
                        .iter()
                        .any(|language| language.contains_normalized(&normalized)))
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
