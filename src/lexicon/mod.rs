mod delete_index;
mod user_lexicon;

pub(crate) use delete_index::{DeleteIndex, MAX_INDEX_DELETIONS};
pub use user_lexicon::{UserLexicon, UserLexiconError, UserTerm, UserTermProtection};

#[cfg(test)]
mod user_lexicon_certification;
#[cfg(test)]
mod user_lexicon_tests;
