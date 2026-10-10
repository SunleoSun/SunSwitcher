use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use fst::{Map, MapBuilder};

pub(crate) const MAX_INDEX_DELETIONS: usize = 2;
const FULL_TYPO_INDEX_MAX_CHARS: usize = 64;
const SINGLE_DELETE_TYPO_INDEX_MAX_CHARS: usize = 128;

#[derive(Debug, Clone)]
enum BuildPostingList {
    One(u32),
    Many(Vec<u32>),
}

impl BuildPostingList {
    fn push(&mut self, index: u32) {
        match self {
            Self::One(existing) => {
                *self = Self::Many(vec![*existing, index]);
            }
            Self::Many(indices) => indices.push(index),
        }
    }

    fn encode(self, many_postings: &mut Vec<Box<[u32]>>) -> u64 {
        match self {
            Self::One(index) => u64::from(index) << 1,
            Self::Many(mut indices) => {
                indices.sort_unstable();
                indices.dedup();
                if let [only] = indices.as_slice() {
                    return u64::from(*only) << 1;
                }
                let posting_index = u64::try_from(many_postings.len())
                    .expect("delete-index posting count exceeds u64 capacity");
                many_postings.push(indices.into_boxed_slice());
                (posting_index << 1) | 1
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DeleteIndex {
    max_deletions: usize,
    forms: Map<Vec<u8>>,
    many_postings: Vec<Box<[u32]>>,
}

impl DeleteIndex {
    pub(crate) fn build<'a>(
        terms: impl IntoIterator<Item = (usize, &'a str)>,
        max_deletions: usize,
    ) -> Self {
        Self::build_with_term_deletions(
            terms
                .into_iter()
                .map(|(index, term)| (index, term, max_deletions)),
            max_deletions,
        )
    }

    pub(crate) fn build_bounded<'a>(
        terms: impl IntoIterator<Item = (usize, &'a str)>,
        max_deletions: usize,
    ) -> Self {
        Self::build_with_term_deletions(
            terms
                .into_iter()
                .map(|(index, term)| (index, term, deletion_budget(term, max_deletions))),
            max_deletions,
        )
    }

    fn build_with_term_deletions<'a>(
        terms: impl IntoIterator<Item = (usize, &'a str, usize)>,
        max_deletions: usize,
    ) -> Self {
        let mut forms = HashMap::<Box<str>, BuildPostingList>::new();
        for (index, term, term_max_deletions) in terms {
            let index =
                u32::try_from(index).expect("delete-index entry count exceeds u32 capacity");
            for form in deletion_forms(term, term_max_deletions.min(max_deletions)) {
                match forms.entry(form.into_boxed_str()) {
                    Entry::Vacant(slot) => {
                        slot.insert(BuildPostingList::One(index));
                    }
                    Entry::Occupied(mut slot) => slot.get_mut().push(index),
                }
            }
        }

        let mut sorted_forms = forms.into_iter().collect::<Vec<_>>();
        sorted_forms.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        let mut builder = MapBuilder::memory();
        let mut many_postings = Vec::new();
        for (form, postings) in sorted_forms {
            let encoded = postings.encode(&mut many_postings);
            builder
                .insert(form.as_ref(), encoded)
                .expect("sorted unique delete forms must build a valid FST");
        }
        let bytes = builder
            .into_inner()
            .expect("in-memory delete-index FST must finalize");
        let forms = Map::new(bytes).expect("generated delete-index FST must be valid");

        Self {
            max_deletions,
            forms,
            many_postings,
        }
    }

    pub(crate) fn candidate_indices(&self, observed: &str, max_deletions: usize) -> Vec<usize> {
        let max_deletions = deletion_budget(observed, max_deletions.min(self.max_deletions));

        let mut indices = HashSet::new();
        for form in deletion_forms(observed, max_deletions) {
            let Some(encoded) = self.forms.get(form.as_str()) else {
                continue;
            };
            if encoded & 1 == 0 {
                indices.insert((encoded >> 1) as usize);
            } else {
                let posting_index = (encoded >> 1) as usize;
                indices.extend(
                    self.many_postings[posting_index]
                        .iter()
                        .map(|&index| index as usize),
                );
            }
        }
        let mut indices: Vec<_> = indices.into_iter().collect();
        indices.sort_unstable();
        indices
    }
}

fn deletion_budget(word: &str, requested: usize) -> usize {
    let chars = word.chars().count();
    if chars <= FULL_TYPO_INDEX_MAX_CHARS {
        requested
    } else if chars <= SINGLE_DELETE_TYPO_INDEX_MAX_CHARS {
        requested.min(1)
    } else {
        0
    }
}

fn deletion_forms(word: &str, max_deletions: usize) -> HashSet<String> {
    let mut seen = HashSet::from([word.to_owned()]);
    let mut frontier = vec![word.to_owned()];

    for _ in 0..max_deletions {
        let mut next = Vec::new();
        for value in frontier {
            let characters: Vec<char> = value.chars().collect();
            for skip in 0..characters.len() {
                let candidate: String = characters
                    .iter()
                    .enumerate()
                    .filter_map(|(index, character)| (index != skip).then_some(*character))
                    .collect();
                if seen.insert(candidate.clone()) {
                    next.push(candidate);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }

    seen
}

#[cfg(test)]
mod tests {
    use super::DeleteIndex;

    #[test]
    fn compact_postings_preserve_unique_and_shared_candidates() {
        let index = DeleteIndex::build([(0, "cat"), (1, "car"), (2, "dog")], 2);
        assert_eq!(index.candidate_indices("ca", 2), vec![0, 1]);
        assert_eq!(index.candidate_indices("dog", 2), vec![2]);
    }

    #[test]
    fn bounded_terms_do_not_change_the_full_dictionary_builder() {
        let term = format!("{}bc", "a".repeat(63));
        let observed = "a".repeat(63);

        let full = DeleteIndex::build([(0, term.as_str())], 2);
        assert_eq!(full.candidate_indices(&observed, 2), vec![0]);

        let bounded = DeleteIndex::build_bounded([(0, term.as_str())], 2);
        assert!(bounded.candidate_indices(&observed, 2).is_empty());
    }

    #[test]
    fn fst_delete_index_matches_reference_form_intersection() {
        let terms = ["cat", "car", "cart", "hello", "для", "AA33", "aaaa"];
        let index = DeleteIndex::build(terms.iter().copied().enumerate(), 2);

        for observed in ["ca", "cta", "carr", "helo", "дял", "AA3", "aaa", "zzz"] {
            let observed_forms = super::deletion_forms(observed, 2);
            let expected = terms
                .iter()
                .enumerate()
                .filter_map(|(term_index, term)| {
                    super::deletion_forms(term, 2)
                        .iter()
                        .any(|form| observed_forms.contains(form))
                        .then_some(term_index)
                })
                .collect::<Vec<_>>();
            assert_eq!(index.candidate_indices(observed, 2), expected, "{observed}");
        }
    }
}
