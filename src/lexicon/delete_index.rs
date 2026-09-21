use std::collections::{HashMap, HashSet};

pub(crate) const MAX_INDEX_DELETIONS: usize = 2;

#[derive(Debug, Clone)]
pub(crate) struct DeleteIndex {
    max_deletions: usize,
    forms: HashMap<String, Vec<usize>>,
}

impl DeleteIndex {
    pub(crate) fn build<'a>(
        terms: impl IntoIterator<Item = (usize, &'a str)>,
        max_deletions: usize,
    ) -> Self {
        let mut forms: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, term) in terms {
            for form in deletion_forms(term, max_deletions) {
                forms.entry(form).or_default().push(index);
            }
        }
        for indices in forms.values_mut() {
            indices.sort_unstable();
            indices.dedup();
        }
        Self {
            max_deletions,
            forms,
        }
    }

    pub(crate) fn candidate_indices(&self, observed: &str, max_deletions: usize) -> Vec<usize> {
        let mut indices = HashSet::new();
        for form in deletion_forms(observed, max_deletions.min(self.max_deletions)) {
            if let Some(matches) = self.forms.get(&form) {
                indices.extend(matches.iter().copied());
            }
        }
        let mut indices: Vec<_> = indices.into_iter().collect();
        indices.sort_unstable();
        indices
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
