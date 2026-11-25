use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use crate::language::normalize_word;

use super::delete_index::{DeleteIndex, MAX_INDEX_DELETIONS, deletion_budget, deletion_forms};

const MAX_INCREMENTAL_OVERLAY_ENTRIES: usize = 256;

#[derive(Debug, Clone, Default)]
struct UserLexiconOverlay {
    entries: Vec<Option<Arc<UserWord>>>,
    exact_index: HashMap<Arc<str>, usize>,
    delete_forms: HashMap<Box<str>, Vec<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserWord {
    term: Box<str>,
    normalized_term: Arc<str>,
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
            term: term.into_boxed_str(),
            normalized_term: Arc::from(normalized_term),
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
    entries: Vec<Arc<UserWord>>,
    exact_index: Arc<HashMap<Arc<str>, usize>>,
    delete_index: Arc<DeleteIndex>,
    removed: Arc<HashSet<usize>>,
    overlay: Arc<UserLexiconOverlay>,
    max_use_count: u32,
}

impl UserLexicon {
    pub fn try_new(mut entries: Vec<UserWord>) -> Result<Self, UserLexiconError> {
        entries.sort_by(|left, right| left.normalized_term.cmp(&right.normalized_term));
        let entries = entries.into_iter().map(Arc::new).collect::<Vec<_>>();

        let mut exact_index = HashMap::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            if exact_index
                .insert(Arc::clone(&entry.normalized_term), index)
                .is_some()
            {
                return Err(UserLexiconError::DuplicateNormalizedTerm(
                    entry.normalized_term.to_string(),
                ));
            }
        }

        let delete_index = DeleteIndex::build_bounded(
            entries
                .iter()
                .enumerate()
                .map(|(index, entry)| (index, entry.normalized_term())),
            MAX_INDEX_DELETIONS,
        );
        let max_use_count = entries
            .iter()
            .map(|entry| entry.use_count())
            .max()
            .unwrap_or(1);

        Ok(Self {
            entries,
            exact_index: Arc::new(exact_index),
            delete_index: Arc::new(delete_index),
            removed: Arc::new(HashSet::new()),
            overlay: Arc::new(UserLexiconOverlay::default()),
            max_use_count,
        })
    }

    pub fn with_updated_existing_many(
        &self,
        updates: impl IntoIterator<Item = UserWord>,
    ) -> Option<Self> {
        let mut entries = self.entries.clone();
        let mut max_use_count = self.max_use_count;
        for mut update in updates {
            let index = *self.exact_index.get(update.normalized_term())?;
            if self.removed.contains(&index) {
                return None;
            }
            max_use_count = max_use_count.max(update.use_count());
            update.normalized_term = Arc::clone(&self.entries[index].normalized_term);
            entries[index] = Arc::new(update);
        }
        Some(Self {
            entries,
            exact_index: Arc::clone(&self.exact_index),
            delete_index: Arc::clone(&self.delete_index),
            removed: Arc::clone(&self.removed),
            overlay: Arc::clone(&self.overlay),
            max_use_count,
        })
    }

    pub fn with_upserted_many(
        &self,
        updates: impl IntoIterator<Item = UserWord>,
    ) -> Option<Self> {
        let mut entries = self.entries.clone();
        let mut removed = (*self.removed).clone();
        let mut overlay = (*self.overlay).clone();
        let mut max_use_count = self.max_use_count;
        let mut removed_changed = false;
        let mut overlay_changed = false;

        for mut update in updates {
            max_use_count = max_use_count.max(update.use_count());
            if let Some(&index) = self.exact_index.get(update.normalized_term()) {
                if removed.remove(&index) {
                    removed_changed = true;
                }
                update.normalized_term = Arc::clone(&self.entries[index].normalized_term);
                entries[index] = Arc::new(update);
            } else if let Some(&ov_index) = overlay.exact_index.get(update.normalized_term()) {
                overlay.entries[ov_index] = Some(Arc::new(update));
                overlay_changed = true;
            } else {
                if overlay.entries.len() >= MAX_INCREMENTAL_OVERLAY_ENTRIES {
                    return None;
                }
                let ov_index = overlay.entries.len();
                let norm = Arc::clone(&update.normalized_term);
                let budget = deletion_budget(norm.as_ref(), MAX_INDEX_DELETIONS);
                for form in deletion_forms(norm.as_ref(), budget) {
                    overlay
                        .delete_forms
                        .entry(form.into_boxed_str())
                        .or_default()
                        .push(ov_index);
                }
                overlay.exact_index.insert(norm, ov_index);
                overlay.entries.push(Some(Arc::new(update)));
                overlay_changed = true;
            }
        }

        Some(Self {
            entries,
            exact_index: Arc::clone(&self.exact_index),
            delete_index: Arc::clone(&self.delete_index),
            removed: if removed_changed {
                Arc::new(removed)
            } else {
                Arc::clone(&self.removed)
            },
            overlay: if overlay_changed {
                Arc::new(overlay)
            } else {
                Arc::clone(&self.overlay)
            },
            max_use_count,
        })
    }

    pub fn without_normalized(&self, normalized_term: &str) -> Self {
        if let Some(&index) = self.exact_index.get(normalized_term) {
            if self.removed.contains(&index) {
                return self.clone();
            }
            let mut removed = (*self.removed).clone();
            removed.insert(index);
            let needs_max_recompute = self.entries[index].use_count() >= self.max_use_count;
            let mut updated = Self {
                entries: self.entries.clone(),
                exact_index: Arc::clone(&self.exact_index),
                delete_index: Arc::clone(&self.delete_index),
                removed: Arc::new(removed),
                overlay: Arc::clone(&self.overlay),
                max_use_count: self.max_use_count,
            };
            if needs_max_recompute {
                updated.max_use_count = updated.recompute_max_use_count();
            }
            return updated;
        }
        if let Some(&ov_index) = self.overlay.exact_index.get(normalized_term)
            && let Some(existing) = self.overlay.entries[ov_index].as_ref()
        {
            let needs_max_recompute = existing.use_count() >= self.max_use_count;
            let mut overlay = (*self.overlay).clone();
            overlay.entries[ov_index] = None;
            let mut updated = Self {
                entries: self.entries.clone(),
                exact_index: Arc::clone(&self.exact_index),
                delete_index: Arc::clone(&self.delete_index),
                removed: Arc::clone(&self.removed),
                overlay: Arc::new(overlay),
                max_use_count: self.max_use_count,
            };
            if needs_max_recompute {
                updated.max_use_count = updated.recompute_max_use_count();
            }
            return updated;
        }
        self.clone()
    }

    fn recompute_max_use_count(&self) -> u32 {
        let base_max = self
            .entries
            .iter()
            .enumerate()
            .filter(|(index, _)| !self.removed.contains(index))
            .map(|(_, entry)| entry.use_count())
            .max()
            .unwrap_or(1);
        let overlay_max = self
            .overlay
            .entries
            .iter()
            .flatten()
            .map(|entry| entry.use_count())
            .max()
            .unwrap_or(1);
        base_max.max(overlay_max)
    }

    pub fn empty() -> Self {
        Self::try_new(Vec::new()).expect("empty user lexicon is valid")
    }

    pub fn contains_normalized(&self, normalized_term: &str) -> bool {
        self.exact(normalized_term).is_some()
    }

    pub fn exact(&self, normalized_term: &str) -> Option<&UserWord> {
        if let Some(&index) = self.exact_index.get(normalized_term) {
            return (!self.removed.contains(&index)).then(|| self.entries[index].as_ref());
        }
        self.overlay
            .exact_index
            .get(normalized_term)
            .and_then(|&ov_index| self.overlay.entries[ov_index].as_deref())
    }

    pub fn candidate_entries(&self, observed: &str, max_deletions: usize) -> Vec<&UserWord> {
        let mut results: Vec<&UserWord> = self
            .delete_index
            .candidate_indices(observed, max_deletions)
            .into_iter()
            .filter(|index| !self.removed.contains(index))
            .map(|index| self.entries[index].as_ref())
            .collect();
        if !self.overlay.entries.is_empty() {
            let budget = deletion_budget(observed, max_deletions.min(MAX_INDEX_DELETIONS));
            let mut seen_overlay = HashSet::new();
            for form in deletion_forms(observed, budget) {
                if let Some(indices) = self.overlay.delete_forms.get(form.as_str()) {
                    for &ov_index in indices {
                        if seen_overlay.insert(ov_index)
                            && let Some(entry) = self.overlay.entries[ov_index].as_deref()
                        {
                            results.push(entry);
                        }
                    }
                }
            }
        }
        results
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
            .partition_point(|entry| entry.normalized_term.as_ref() < normalized_prefix.as_str());
        let mut matches: Vec<_> = self.entries[start..]
            .iter()
            .enumerate()
            .take_while(|(_, entry)| entry.normalized_term.starts_with(&normalized_prefix))
            .filter(|(offset, _)| !self.removed.contains(&(start + offset)))
            .map(|(_, entry)| entry.as_ref())
            .collect();
        if !self.overlay.entries.is_empty() {
            matches.extend(
                self.overlay
                    .entries
                    .iter()
                    .flatten()
                    .filter(|entry| entry.normalized_term.starts_with(&normalized_prefix))
                    .map(Arc::as_ref),
            );
        }
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
        self.entries.len() == self.removed.len()
            && self.overlay.entries.iter().all(Option::is_none)
    }
}

impl Default for UserLexicon {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod incremental_update_tests {
    use std::sync::Arc;

    use super::{UserLexicon, UserWord};

    #[test]
    fn incremental_upsert_and_removal_preserve_exact_candidates_and_prefixes() {
        let base = UserLexicon::try_new(vec![
            UserWord::try_new("alpha", 5, 10).unwrap(),
            UserWord::try_new("alpine", 2, 20).unwrap(),
        ])
        .unwrap();

        let added = base
            .with_upserted_many([UserWord::try_new("джоба", 1, 30).unwrap()])
            .expect("small overlay upsert should stay incremental");
        assert!(added.contains_normalized("джоба"));
        assert_eq!(added.exact("джоба").unwrap().term(), "джоба");
        assert!(
            added
                .candidate_entries("джоб", 2)
                .iter()
                .any(|w| w.normalized_term() == "джоба")
        );
        assert_eq!(added.prefix_matches("джо", 5).len(), 1);

        let removed_base = added.without_normalized("alpha");
        assert!(!removed_base.contains_normalized("alpha"));
        assert!(removed_base.exact("alpha").is_none());
        assert!(
            !removed_base
                .candidate_entries("alph", 2)
                .iter()
                .any(|w| w.normalized_term() == "alpha")
        );
        assert_eq!(removed_base.max_use_count(), 2);

        let removed_overlay = removed_base.without_normalized("джоба");
        assert!(!removed_overlay.contains_normalized("джоба"));
        assert!(removed_overlay.prefix_matches("джо", 5).is_empty());

        let restored = removed_base
            .with_upserted_many([UserWord::try_new("alpha", 7, 40).unwrap()])
            .expect("restoring a tombstoned base word should stay incremental");
        assert!(restored.contains_normalized("alpha"));
        assert_eq!(restored.exact("alpha").unwrap().use_count(), 7);
        assert_eq!(restored.max_use_count(), 7);
    }

    #[test]
    fn metadata_refresh_reuses_search_indexes_for_existing_words() {
        let lexicon = UserLexicon::try_new(vec![
            UserWord::try_new("sun", 1, 10).unwrap(),
            UserWord::try_new("moon", 2, 20).unwrap(),
        ])
        .unwrap();
        let updated = lexicon
            .with_updated_existing_many(vec![UserWord::try_new("sun", 7, 30).unwrap()])
            .unwrap();

        assert!(Arc::ptr_eq(&lexicon.exact_index, &updated.exact_index));
        assert!(Arc::ptr_eq(&lexicon.delete_index, &updated.delete_index));
        assert_eq!(updated.exact("sun").unwrap().use_count(), 7);
        assert_eq!(updated.exact("sun").unwrap().last_used_at_ms(), 30);
        assert!(!updated.candidate_entries("snu", 2).is_empty());
    }

    #[test]
    fn metadata_refresh_rejects_a_new_word_so_the_caller_can_rebuild_indexes() {
        let lexicon = UserLexicon::try_new(vec![UserWord::try_new("sun", 1, 10).unwrap()]).unwrap();
        assert!(
            lexicon
                .with_updated_existing_many(vec![UserWord::try_new("moon", 1, 20).unwrap()])
                .is_none()
        );
    }
}
