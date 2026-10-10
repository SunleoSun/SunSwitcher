mod delete_index;
mod ignored_words;
mod user_lexicon;

pub(crate) use delete_index::{DeleteIndex, MAX_INDEX_DELETIONS};
pub use ignored_words::IgnoredWords;
pub use user_lexicon::{UserLexicon, UserLexiconError, UserWord};

#[cfg(test)]
mod user_lexicon_certification;
#[cfg(test)]
mod user_lexicon_tests;
