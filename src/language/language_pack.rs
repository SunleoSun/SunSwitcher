use std::collections::HashMap;
use std::sync::Arc;

use fst::{IntoStreamer, Map, MapBuilder, Streamer};

use crate::input::PhysicalKey;
use crate::lexicon::{DeleteIndex, MAX_INDEX_DELETIONS};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LanguageId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguagePackError {
    InvalidLanguageId,
    InvalidDictionaryEntry,
    InvalidDictionaryAsset,
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

    pub fn transform_text(&self, text: &str) -> Option<String> {
        let mut transformed = String::with_capacity(text.len());
        let mut changed = false;
        for character in text.chars() {
            if let Some(mapped) = self.map.get(&character).copied() {
                changed |= mapped != character;
                transformed.push(mapped);
            } else if character.is_alphabetic() {
                return None;
            } else {
                transformed.push(character);
            }
        }
        changed.then_some(transformed)
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
            } else if character.is_alphabetic() {
                return None;
            } else {
                character
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

#[derive(Debug)]
struct LanguageDictionaryData {
    dictionary: Map<Vec<u8>>,
    correction_entries: Vec<DictionaryEntry>,
    delete_index: DeleteIndex,
    max_frequency: u32,
}

#[derive(Debug, Clone)]
pub struct LanguagePack {
    id: LanguageId,
    data: Arc<LanguageDictionaryData>,
    transforms: Vec<KeyboardLayoutMap>,
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
        let dictionary = build_dictionary_map(&entries)?;
        Self::from_dictionary(id, dictionary, entries, transforms)
    }

    pub fn try_from_fst_bytes(
        id: LanguageId,
        dictionary_bytes: Vec<u8>,
        correction_bytes: Vec<u8>,
        transforms: Vec<KeyboardLayoutMap>,
    ) -> Result<Self, LanguagePackError> {
        let dictionary =
            Map::new(dictionary_bytes).map_err(|_| LanguagePackError::InvalidDictionaryAsset)?;
        if dictionary.is_empty() {
            return Err(LanguagePackError::EmptyDictionary);
        }
        let correction_dictionary =
            Map::new(correction_bytes).map_err(|_| LanguagePackError::InvalidDictionaryAsset)?;
        if correction_dictionary.is_empty() {
            return Err(LanguagePackError::InvalidDictionaryAsset);
        }

        let mut correction_entries = Vec::with_capacity(correction_dictionary.len());
        let mut max_frequency = 1_u32;
        let mut stream = correction_dictionary.stream();
        while let Some((key, stored_frequency)) = stream.next() {
            let word =
                std::str::from_utf8(key).map_err(|_| LanguagePackError::InvalidDictionaryAsset)?;
            let frequency = u32::try_from(stored_frequency)
                .map_err(|_| LanguagePackError::InvalidDictionaryAsset)?;
            if frequency == 0 || normalize_word(word) != word {
                return Err(LanguagePackError::InvalidDictionaryAsset);
            }
            if dictionary.get(word) != Some(stored_frequency) {
                return Err(LanguagePackError::InvalidDictionaryAsset);
            }
            max_frequency = max_frequency.max(frequency);
            correction_entries.push(DictionaryEntry {
                word: word.to_owned(),
                frequency,
            });
        }
        Self::from_dictionary_with_max(
            id,
            dictionary,
            correction_entries,
            transforms,
            max_frequency,
        )
    }

    fn from_dictionary(
        id: LanguageId,
        dictionary: Map<Vec<u8>>,
        correction_entries: Vec<DictionaryEntry>,
        transforms: Vec<KeyboardLayoutMap>,
    ) -> Result<Self, LanguagePackError> {
        let max_frequency = correction_entries
            .iter()
            .map(DictionaryEntry::frequency)
            .max()
            .unwrap_or(1);
        Self::from_dictionary_with_max(
            id,
            dictionary,
            correction_entries,
            transforms,
            max_frequency,
        )
    }

    fn from_dictionary_with_max(
        id: LanguageId,
        dictionary: Map<Vec<u8>>,
        correction_entries: Vec<DictionaryEntry>,
        transforms: Vec<KeyboardLayoutMap>,
        max_frequency: u32,
    ) -> Result<Self, LanguagePackError> {
        if correction_entries.is_empty() {
            return Err(LanguagePackError::EmptyDictionary);
        }
        let delete_index = DeleteIndex::build(
            correction_entries
                .iter()
                .enumerate()
                .map(|(index, entry)| (index, entry.word())),
            MAX_INDEX_DELETIONS,
        );
        Ok(Self {
            id,
            data: Arc::new(LanguageDictionaryData {
                dictionary,
                correction_entries,
                delete_index,
                max_frequency,
            }),
            transforms,
        })
    }

    pub fn id(&self) -> &LanguageId {
        &self.id
    }

    pub fn contains_normalized(&self, word: &str) -> bool {
        self.data.dictionary.contains_key(word)
    }

    pub fn exact_entry(&self, normalized_word: &str) -> Option<DictionaryEntry> {
        let stored_frequency = self.data.dictionary.get(normalized_word)?;
        let frequency = u32::try_from(stored_frequency).ok()?;
        Some(DictionaryEntry {
            word: normalized_word.to_owned(),
            frequency,
        })
    }

    pub fn transforms(&self) -> &[KeyboardLayoutMap] {
        &self.transforms
    }

    pub fn max_frequency(&self) -> u32 {
        self.data.max_frequency
    }

    pub fn prefix_matches(&self, prefix: &str, limit: usize) -> Vec<DictionaryEntry> {
        if limit == 0 {
            return Vec::new();
        }
        let normalized_prefix = normalize_word(prefix);
        if normalized_prefix.is_empty() {
            return Vec::new();
        }

        let prefix_bytes = normalized_prefix.as_bytes();
        let mut stream = self.data.dictionary.range().ge(prefix_bytes).into_stream();
        let mut entries: Vec<DictionaryEntry> = Vec::with_capacity(limit);
        while let Some((key, stored_frequency)) = stream.next() {
            if !key.starts_with(prefix_bytes) {
                break;
            }
            let Ok(word) = std::str::from_utf8(key) else {
                break;
            };
            let Ok(frequency) = u32::try_from(stored_frequency) else {
                break;
            };

            if entries.len() == limit {
                let worst = entries.last().expect("limit is non-zero");
                if frequency < worst.frequency()
                    || (frequency == worst.frequency() && word >= worst.word())
                {
                    continue;
                }
            }

            entries.push(DictionaryEntry {
                word: word.to_owned(),
                frequency,
            });
            entries.sort_by(|left, right| {
                right
                    .frequency()
                    .cmp(&left.frequency())
                    .then_with(|| left.word().cmp(right.word()))
            });
            entries.truncate(limit);
        }
        entries
    }

    pub fn candidate_entries(&self, observed: &str, max_deletions: usize) -> Vec<&DictionaryEntry> {
        self.data
            .delete_index
            .candidate_indices(observed, max_deletions)
            .into_iter()
            .map(|index| &self.data.correction_entries[index])
            .collect()
    }
}

fn build_dictionary_map(entries: &[DictionaryEntry]) -> Result<Map<Vec<u8>>, LanguagePackError> {
    let mut builder = MapBuilder::memory();
    for entry in entries {
        builder
            .insert(entry.word(), u64::from(entry.frequency()))
            .map_err(|_| LanguagePackError::InvalidDictionaryAsset)?;
    }
    let bytes = builder
        .into_inner()
        .map_err(|_| LanguagePackError::InvalidDictionaryAsset)?;
    Map::new(bytes).map_err(|_| LanguagePackError::InvalidDictionaryAsset)
}

pub fn normalize_word(word: &str) -> String {
    word.chars().flat_map(char::to_lowercase).collect()
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

    fn fst_bytes(entries: &[(&str, u32)]) -> Vec<u8> {
        let mut builder = MapBuilder::memory();
        for (word, frequency) in entries {
            builder.insert(word, u64::from(*frequency)).unwrap();
        }
        builder.into_inner().unwrap()
    }

    #[test]
    fn fst_dictionary_separates_full_membership_from_correction_candidates() {
        let pack = LanguagePack::try_from_fst_bytes(
            LanguageId::try_new("test").unwrap(),
            fst_bytes(&[("hello", 100), ("help", 90), ("hero", 80), ("zoo", 5)]),
            fst_bytes(&[("hello", 100), ("help", 90)]),
            Vec::new(),
        )
        .unwrap();

        assert!(pack.contains_normalized("zoo"));
        assert!(
            pack.candidate_entries("hellp", 2)
                .iter()
                .any(|entry| entry.word() == "hello")
        );
        assert!(
            pack.candidate_entries("zop", 2)
                .iter()
                .all(|entry| entry.word() != "zoo")
        );
        assert_eq!(
            pack.prefix_matches("he", 2)
                .iter()
                .map(DictionaryEntry::word)
                .collect::<Vec<_>>(),
            ["hello", "help"]
        );
    }

    #[test]
    fn fst_correction_asset_must_match_the_full_dictionary() {
        let result = LanguagePack::try_from_fst_bytes(
            LanguageId::try_new("test").unwrap(),
            fst_bytes(&[("hello", 100)]),
            fst_bytes(&[("hello", 99)]),
            Vec::new(),
        );
        assert!(matches!(
            result,
            Err(LanguagePackError::InvalidDictionaryAsset)
        ));
    }

    #[test]
    fn aligned_layout_map_preserves_case_and_rejects_unmapped_text() {
        let map = KeyboardLayoutMap::from_aligned("lkz", "для", 0.15).unwrap();
        assert_eq!(map.transform("lkz").as_deref(), Some("для"));
        assert_eq!(map.transform("Lkz").as_deref(), Some("Для"));
        assert_eq!(map.transform("lkx"), None);
    }

    #[test]
    fn layout_text_transform_preserves_non_letters_between_mapped_words() {
        let map = KeyboardLayoutMap::from_aligned("ghbdtn", "привет", 0.15).unwrap();
        assert_eq!(
            map.transform_text("ghbdtn 123! ghbdtn").as_deref(),
            Some("привет 123! привет")
        );
        assert_eq!(map.transform_text("ghbdtn x"), None);
    }

    #[test]
    fn physical_layout_transform_preserves_unmapped_digits() {
        let map = KeyboardLayoutMap::from_aligned("фа", "aa", 0.15).unwrap();
        assert_eq!(
            map.transform_with_physical(
                "ФФ33",
                &[
                    PhysicalKey::Other,
                    PhysicalKey::Other,
                    PhysicalKey::Other,
                    PhysicalKey::Other,
                ],
            )
            .as_deref(),
            Some("AA33")
        );
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
