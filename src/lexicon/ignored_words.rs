use std::collections::HashSet;
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct IgnoredWords {
    normalized_terms: Arc<HashSet<Box<str>>>,
}

impl IgnoredWords {
    pub fn from_normalized_words(words: impl IntoIterator<Item = String>) -> Self {
        Self {
            normalized_terms: Arc::new(
                words
                    .into_iter()
                    .map(String::into_boxed_str)
                    .collect::<HashSet<_>>(),
            ),
        }
    }

    pub fn contains_normalized(&self, normalized_term: &str) -> bool {
        self.normalized_terms.contains(normalized_term)
    }

    pub fn is_empty(&self) -> bool {
        self.normalized_terms.is_empty()
    }

    pub fn len(&self) -> usize {
        self.normalized_terms.len()
    }
}
