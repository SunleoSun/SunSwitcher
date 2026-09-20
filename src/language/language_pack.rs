use std::collections::{HashMap, HashSet};

use crate::input::PhysicalKey;

const INDEX_MAX_DELETIONS: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LanguageId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguagePackError {
    InvalidLanguageId,
    InvalidDictionaryEntry,
    InvalidLayoutMap,
    EmptyDictionary,
}

impl LanguageId {
    pub fn try_new(value: impl Into<String>) -> Result<Self, LanguagePackError> {
        let value = value.into();
        if value.trim().is_empty() || value.contains('\0') {
            return Err(LanguagePackError::InvalidLanguageId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictionaryEntry {
    word: String,
    frequency: u32,
}

impl DictionaryEntry {
    pub fn try_new(word: impl Into<String>, frequency: u32) -> Result<Self, LanguagePackError> {
        let word = normalize_word(&word.into());
        if word.is_empty() || word.contains('\0') || frequency == 0 {
            return Err(LanguagePackError::InvalidDictionaryEntry);
        }
        Ok(Self { word, frequency })
    }

    pub fn word(&self) -> &str {
        &self.word
    }

    pub fn frequency(&self) -> u32 {
        self.frequency
    }
}

#[derive(Debug, Clone)]
pub struct KeyboardLayoutMap {
    map: HashMap<char, char>,
    physical_map: HashMap<PhysicalKey, char>,
    penalty: f32,
}

impl KeyboardLayoutMap {
    pub fn from_aligned(
        source: &str,
        target: &str,
        penalty: f32,
    ) -> Result<Self, LanguagePackError> {
        if !penalty.is_finite() || !(0.0..=1.0).contains(&penalty) {
            return Err(LanguagePackError::InvalidLayoutMap);
        }

        let source_chars: Vec<char> = source.chars().collect();
        let target_chars: Vec<char> = target.chars().collect();
        if source_chars.is_empty() || source_chars.len() != target_chars.len() {
            return Err(LanguagePackError::InvalidLayoutMap);
        }

        let mut map = HashMap::new();
        let mut physical_map = HashMap::new();
        for (source_char, target_char) in source_chars.into_iter().zip(target_chars) {
            insert_unique_mapping(&mut map, source_char, target_char)?;

            let source_physical = PhysicalKey::from_layout_symbol(source_char);
            let target_physical = PhysicalKey::from_layout_symbol(target_char);
            if source_physical.is_layout_ambiguous() {
                insert_unique_physical_mapping(&mut physical_map, source_physical, target_char)?;
            }

            if source_char.is_alphabetic()
                && let Some(source_upper) = single_uppercase(source_char)
            {
                let shifted_target = if target_char.is_alphabetic() {
                    single_uppercase(target_char)
                } else {
                    target_physical.shifted_symbol()
                };
                if let Some(shifted_target) = shifted_target {
                    insert_unique_mapping(&mut map, source_upper, shifted_target)?;
                }
            }
        }

        Ok(Self {
            map,
            physical_map,
            penalty,
        })
    }

    pub fn transform(&self, text: &str) -> Option<String> {
        let physical_keys: Vec<_> = text.chars().map(PhysicalKey::from_layout_symbol).collect();
        self.transform_with_physical(text, &physical_keys)
    }

    pub fn transform_with_physical(
        &self,
        text: &str,
        physical_keys: &[PhysicalKey],
    ) -> Option<String> {
        let characters: Vec<_> = text.chars().collect();
        if characters.len() != physical_keys.len() {
            return None;
        }

        let mut transformed = String::with_capacity(text.len());
        let mut changed = false;
        for (character, physical_key) in characters.into_iter().zip(physical_keys.iter().copied()) {
            let mapped = if let Some(mapped) = self.map.get(&character).copied() {
                mapped
            } else if let Some(base_target) = self.physical_map.get(&physical_key).copied() {
                if physical_key.is_shifted_symbol(character) && base_target.is_alphabetic() {
                    single_uppercase(base_target).unwrap_or(base_target)
                } else {
                    base_target
                }
            } else {
                return None;
            };
            changed |= mapped != character;
            transformed.push(mapped);
        }
        changed.then_some(transformed)
    }

    pub fn penalty(&self) -> f32 {
        self.penalty
    }
}

fn insert_unique_mapping(
    map: &mut HashMap<char, char>,
    source: char,
    target: char,
) -> Result<(), LanguagePackError> {
    match map.insert(source, target) {
        Some(previous) if previous != target => Err(LanguagePackError::InvalidLayoutMap),
        _ => Ok(()),
    }
}

fn insert_unique_physical_mapping(
    map: &mut HashMap<PhysicalKey, char>,
    source: PhysicalKey,
    target: char,
) -> Result<(), LanguagePackError> {
    match map.insert(source, target) {
        Some(previous) if previous != target => Err(LanguagePackError::InvalidLayoutMap),
        _ => Ok(()),
    }
}

fn single_uppercase(character: char) -> Option<char> {
    let mut uppercase = character.to_uppercase();
    let first = uppercase.next()?;
    uppercase.next().is_none().then_some(first)
}

#[derive(Debug, Clone)]
pub struct LanguagePack {
    id: LanguageId,
    entries: Vec<DictionaryEntry>,
    exact_index: HashMap<String, usize>,
    delete_index: HashMap<String, Vec<usize>>,
    transforms: Vec<KeyboardLayoutMap>,
    max_frequency: u32,
}

impl LanguagePack {
    pub fn try_new(
        id: LanguageId,
        entries: Vec<DictionaryEntry>,
        transforms: Vec<KeyboardLayoutMap>,
    ) -> Result<Self, LanguagePackError> {
        if entries.is_empty() {
            return Err(LanguagePackError::EmptyDictionary);
        }

        let mut deduplicated: HashMap<String, DictionaryEntry> = HashMap::new();
        for entry in entries {
            deduplicated
                .entry(entry.word.clone())
                .and_modify(|existing| {
                    if entry.frequency > existing.frequency {
                        *existing = entry.clone();
                    }
                })
                .or_insert(entry);
        }

        let mut entries: Vec<_> = deduplicated.into_values().collect();
        entries.sort_by(|left, right| left.word.cmp(&right.word));
        let max_frequency = entries
            .iter()
            .map(DictionaryEntry::frequency)
            .max()
            .unwrap_or(1);

        let mut exact_index = HashMap::new();
        let mut delete_index: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, entry) in entries.iter().enumerate() {
            exact_index.insert(entry.word.clone(), index);
            for form in deletion_forms(entry.word(), INDEX_MAX_DELETIONS) {
                delete_index.entry(form).or_default().push(index);
            }
        }

        Ok(Self {
            id,
            entries,
            exact_index,
            delete_index,
            transforms,
            max_frequency,
        })
    }

    pub fn id(&self) -> &LanguageId {
        &self.id
    }

    pub fn contains_normalized(&self, word: &str) -> bool {
        self.exact_index.contains_key(word)
    }

    pub fn transforms(&self) -> &[KeyboardLayoutMap] {
        &self.transforms
    }

    pub fn max_frequency(&self) -> u32 {
        self.max_frequency
    }

    pub fn candidate_entries(&self, observed: &str, max_deletions: usize) -> Vec<&DictionaryEntry> {
        let mut indices = HashSet::new();
        for form in deletion_forms(observed, max_deletions.min(INDEX_MAX_DELETIONS)) {
            if let Some(matches) = self.delete_index.get(&form) {
                indices.extend(matches.iter().copied());
            }
        }

        let mut indices: Vec<_> = indices.into_iter().collect();
        indices.sort_unstable();
        indices
            .into_iter()
            .map(|index| &self.entries[index])
            .collect()
    }
}

pub fn normalize_word(word: &str) -> String {
    word.chars().flat_map(char::to_lowercase).collect()
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
    use super::*;

    #[test]
    fn delete_index_finds_transposition_missing_and_duplicate_neighbors() {
        let pack = LanguagePack::try_new(
            LanguageId::try_new("test").unwrap(),
            vec![DictionaryEntry::try_new("для", 100).unwrap()],
            Vec::new(),
        )
        .unwrap();

        for observed in ["дял", "дляя", "дл"] {
            assert!(
                pack.candidate_entries(observed, 2)
                    .iter()
                    .any(|entry| entry.word() == "для")
            );
        }
    }

    #[test]
    fn aligned_layout_map_preserves_case_and_rejects_unmapped_text() {
        let map = KeyboardLayoutMap::from_aligned("lkz", "для", 0.15).unwrap();
        assert_eq!(map.transform("lkz").as_deref(), Some("для"));
        assert_eq!(map.transform("Lkz").as_deref(), Some("Для"));
        assert_eq!(map.transform("lkx"), None);
    }

    #[test]
    fn aligned_layout_map_preserves_shift_semantics_when_target_is_punctuation() {
        let russian_to_latin = KeyboardLayoutMap::from_aligned("бюжэ", ",.;'", 0.15).unwrap();
        for (source, expected) in [
            ("б", ","),
            ("Б", "<"),
            ("ю", "."),
            ("Ю", ">"),
            ("ж", ";"),
            ("Ж", ":"),
            ("э", "'"),
            ("Э", "\""),
        ] {
            assert_eq!(
                russian_to_latin.transform(source).as_deref(),
                Some(expected)
            );
        }

        let latin_to_russian = KeyboardLayoutMap::from_aligned(";", "ж", 0.15).unwrap();
        assert_eq!(
            latin_to_russian
                .transform_with_physical(":", &[PhysicalKey::Semicolon])
                .as_deref(),
            Some("Ж")
        );
    }
}
