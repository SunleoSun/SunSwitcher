use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::language::normalize_word;

use super::{DeleteIndex, MAX_INDEX_DELETIONS};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserWord {
    term: String,
    normalized_term: String,
    use_count: u32,
    last_used_at_ms: i64,
}

impl UserWord {
    pub fn try_new(
        term: impl Into<String>,
        use_count: u32,
        last_used_at_ms: i64,
    ) -> Result<Self, UserLexiconError> {
        let term = term.into();
        if term.is_empty()
            || term.contains('\0')
            || term.chars().any(char::is_whitespace)
            || !term.chars().any(char::is_alphanumeric)
        {
            return Err(UserLexiconError::InvalidTerm);
        }
        if use_count == 0 {
            return Err(UserLexiconError::InvalidUseCount);
        }
        let normalized_term = normalize_word(&term);
        if normalized_term.is_empty() {
            return Err(UserLexiconError::InvalidTerm);
        }
        Ok(Self {
            term,
            normalized_term,
            use_count,
            last_used_at_ms,
        })
    }

    pub fn term(&self) -> &str {
        &self.term
    }

    pub fn normalized_term(&self) -> &str {
        &self.normalized_term
    }

    pub const fn use_count(&self) -> u32 {
        self.use_count
    }

    pub const fn last_used_at_ms(&self) -> i64 {
        self.last_used_at_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserLexiconError {
    InvalidTerm,
    InvalidUseCount,
    DuplicateNormalizedTerm(String),
}

impl Display for UserLexiconError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTerm => formatter.write_str("invalid user word"),
            Self::InvalidUseCount => formatter.write_str("user word use count must be positive"),
            Self::DuplicateNormalizedTerm(term) => {
                write!(formatter, "duplicate normalized user word: {term:?}")
            }
        }
    }
}

impl Error for UserLexiconError {}

#[derive(Debug, Clone)]
pub struct UserLexicon {
    entries: Vec<UserWord>,
    exact_index: HashMap<String, usize>,
    delete_index: DeleteIndex,
    max_use_count: u32,
}

impl UserLexicon {
    pub fn try_new(mut entries: Vec<UserWord>) -> Result<Self, UserLexiconError> {
        entries.sort_by(|left, right| left.normalized_term.cmp(&right.normalized_term));

        let mut exact_index = HashMap::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            if exact_index
                .insert(entry.normalized_term.clone(), index)
                .is_some()
            {
                return Err(UserLexiconError::DuplicateNormalizedTerm(
                    entry.normalized_term.clone(),
                ));
            }
        }

        let delete_index = DeleteIndex::build(
            entries
                .iter()
                .enumerate()
                .map(|(index, entry)| (index, entry.normalized_term())),
            MAX_INDEX_DELETIONS,
        );
        let max_use_count = entries.iter().map(UserWord::use_count).max().unwrap_or(1);

        Ok(Self {
            entries,
            exact_index,
            delete_index,
            max_use_count,
        })
    }

    pub fn empty() -> Self {
        Self::try_new(Vec::new()).expect("empty user lexicon is valid")
    }

    pub fn contains_normalized(&self, normalized_term: &str) -> bool {
        self.exact_index.contains_key(normalized_term)
    }

    pub fn exact(&self, normalized_term: &str) -> Option<&UserWord> {
        self.exact_index
            .get(normalized_term)
            .map(|index| &self.entries[*index])
    }

    pub fn candidate_entries(&self, observed: &str, max_deletions: usize) -> Vec<&UserWord> {
        self.delete_index
            .candidate_indices(observed, max_deletions)
            .into_iter()
            .map(|index| &self.entries[index])
            .collect()
    }

    pub fn prefix_matches(&self, prefix: &str, limit: usize) -> Vec<&UserWord> {
        if limit == 0 {
            return Vec::new();
        }
        let normalized_prefix = normalize_word(prefix);
        if normalized_prefix.is_empty() {
            return Vec::new();
        }

        let start = self
            .entries
            .partition_point(|entry| entry.normalized_term.as_str() < normalized_prefix.as_str());
        let mut matches: Vec<_> = self.entries[start..]
            .iter()
            .take_while(|entry| entry.normalized_term.starts_with(&normalized_prefix))
            .collect();
        matches.sort_by(|left, right| {
            right
                .last_used_at_ms
                .cmp(&left.last_used_at_ms)
                .then_with(|| right.use_count.cmp(&left.use_count))
                .then_with(|| left.term.cmp(&right.term))
        });
        matches.truncate(limit);
        matches
    }

    pub const fn max_use_count(&self) -> u32 {
        self.max_use_count
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for UserLexicon {
    fn default() -> Self {
        Self::empty()
    }
}
