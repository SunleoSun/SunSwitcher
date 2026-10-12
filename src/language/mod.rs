mod builtin;
mod english;
mod language_pack;
mod russian;

pub(crate) use builtin::builtin_language_pack;
pub use language_pack::{
    DictionaryEntry, KeyboardLayoutMap, LanguageId, LanguagePack, LanguagePackError, normalize_word,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardLayoutSwitch {
    text: String,
    target_language: LanguageId,
}

impl KeyboardLayoutSwitch {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn target_language(&self) -> &LanguageId {
        &self.target_language
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextCasePattern {
    Lower,
    Upper,
    Title,
    Mixed,
}

impl TextCasePattern {
    pub fn detect(text: &str) -> Self {
        let letters: Vec<char> = text
            .chars()
            .filter(|character| character.is_alphabetic())
            .collect();
        if letters.is_empty() || letters.iter().all(|character| character.is_lowercase()) {
            return Self::Lower;
        }
        if letters.iter().all(|character| character.is_uppercase()) {
            return Self::Upper;
        }
        if letters[0].is_uppercase()
            && letters[1..]
                .iter()
                .all(|character| character.is_lowercase())
        {
            return Self::Title;
        }
        Self::Mixed
    }

    pub fn apply(self, canonical: &str) -> String {
        match self {
            Self::Lower => canonical.chars().flat_map(char::to_lowercase).collect(),
            Self::Upper => canonical.chars().flat_map(char::to_uppercase).collect(),
            Self::Title => {
                let mut characters = canonical.chars();
                let Some(first) = characters.next() else {
                    return String::new();
                };
                first
                    .to_uppercase()
                    .chain(characters.flat_map(char::to_lowercase))
                    .collect()
            }
            Self::Mixed => canonical.to_owned(),
        }
    }
}

pub fn switch_keyboard_layout_text(
    languages: &[LanguagePack],
    text: &str,
) -> Option<KeyboardLayoutSwitch> {
    switch_keyboard_layout_text_with_target(languages, text, None)
}

pub fn switch_keyboard_layout_text_to(
    languages: &[LanguagePack],
    text: &str,
    target_language: &LanguageId,
) -> Option<KeyboardLayoutSwitch> {
    switch_keyboard_layout_text_with_target(languages, text, Some(target_language))
}

fn switch_keyboard_layout_text_with_target(
    languages: &[LanguagePack],
    text: &str,
    preferred_target: Option<&LanguageId>,
) -> Option<KeyboardLayoutSwitch> {
    struct CharacterTransform {
        original: char,
        candidates: Vec<(String, LanguageId)>,
    }

    let mut character_transforms = Vec::with_capacity(text.chars().count());
    let mut target_votes: Vec<(LanguageId, usize)> = Vec::new();

    for character in text.chars() {
        let source = character.to_string();
        let mut candidates = Vec::new();
        for language in languages {
            for transform in language.transforms() {
                let Some(transformed) = transform.transform_text(&source) else {
                    continue;
                };
                let candidate = (transformed, language.id().clone());
                if !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
        }

        if candidates.is_empty() && character.is_alphabetic() {
            return None;
        }
        if candidates.len() == 1 {
            let target_language = &candidates[0].1;
            if let Some((_, votes)) = target_votes
                .iter_mut()
                .find(|(language, _)| language == target_language)
            {
                *votes += 1;
            } else {
                target_votes.push((target_language.clone(), 1));
            }
        }
        character_transforms.push(CharacterTransform {
            original: character,
            candidates,
        });
    }

    let target_language = if let Some(target_language) = preferred_target {
        languages
            .iter()
            .any(|language| language.id() == target_language)
            .then(|| target_language.clone())?
    } else {
        let best_votes = target_votes.iter().map(|(_, votes)| *votes).max()?;
        let mut winners = target_votes
            .into_iter()
            .filter(|(_, votes)| *votes == best_votes);
        let (target_language, _) = winners.next()?;
        if winners.next().is_some() {
            return None;
        }
        target_language
    };

    let mut transformed_text = String::with_capacity(text.len());
    let mut changed = false;
    for character in character_transforms {
        let transformed = if preferred_target.is_some() {
            match character.candidates.as_slice() {
                [] => character.original.to_string(),
                candidates => {
                    let mut matching = candidates
                        .iter()
                        .filter(|(_, language)| language == &target_language);
                    let (transformed, _) = matching.next()?;
                    if matching.next().is_some() {
                        return None;
                    }
                    transformed.clone()
                }
            }
        } else {
            match character.candidates.as_slice() {
                [] => character.original.to_string(),
                [(transformed, _)] => transformed.clone(),
                candidates => {
                    let mut matching = candidates
                        .iter()
                        .filter(|(_, language)| language == &target_language);
                    let (transformed, _) = matching.next()?;
                    if matching.next().is_some() {
                        return None;
                    }
                    transformed.clone()
                }
            }
        };
        changed |= transformed != character.original.to_string();
        transformed_text.push_str(&transformed);
    }

    changed.then_some(KeyboardLayoutSwitch {
        text: transformed_text,
        target_language,
    })
}

pub fn language_pack_from_entries(
    id: LanguageId,
    entries: Vec<DictionaryEntry>,
) -> Result<LanguagePack, LanguagePackError> {
    let transforms = layout_transforms_for(&id)?;
    LanguagePack::try_new(id, entries, transforms)
}

pub(crate) fn language_pack_from_fst_bytes(
    id: LanguageId,
    dictionary_bytes: Vec<u8>,
    correction_bytes: Vec<u8>,
) -> Result<LanguagePack, LanguagePackError> {
    let transforms = layout_transforms_for(&id)?;
    LanguagePack::try_from_fst_bytes(id, dictionary_bytes, correction_bytes, transforms)
}

fn layout_transforms_for(id: &LanguageId) -> Result<Vec<KeyboardLayoutMap>, LanguagePackError> {
    match id.as_str() {
        "en" => english::layout_transforms(),
        "ru" => russian::layout_transforms(),
        _ => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DictionaryEntry, LanguageId, TextCasePattern, language_pack_from_entries,
        switch_keyboard_layout_text, switch_keyboard_layout_text_to,
    };

    #[test]
    fn layout_switch_carries_the_target_language_with_transformed_text() {
        let english = language_pack_from_entries(
            LanguageId::try_new("en").unwrap(),
            vec![DictionaryEntry::try_new("hello", 1).unwrap()],
        )
        .unwrap();
        let russian = language_pack_from_entries(
            LanguageId::try_new("ru").unwrap(),
            vec![DictionaryEntry::try_new("привет", 1).unwrap()],
        )
        .unwrap();

        let to_russian = switch_keyboard_layout_text(&[english.clone(), russian.clone()], "ghbdtn")
            .expect("latin physical spelling should map to Russian");
        assert_eq!(to_russian.text(), "привет");
        assert_eq!(to_russian.target_language().as_str(), "ru");

        let punctuation_to_russian =
            switch_keyboard_layout_text(&[english.clone(), russian.clone()], "ndj.")
                .expect("layout punctuation before whitespace belongs to the switched span");
        assert_eq!(punctuation_to_russian.text(), "твою");
        assert_eq!(punctuation_to_russian.target_language().as_str(), "ru");
        assert_eq!(
            switch_keyboard_layout_text(&[english.clone(), russian.clone()], "lkz/")
                .unwrap()
                .text(),
            "для."
        );
        assert_eq!(
            switch_keyboard_layout_text(&[english.clone(), russian.clone()], "ndj/")
                .unwrap()
                .text(),
            "тво."
        );
        assert_eq!(
            switch_keyboard_layout_text(&[english.clone(), russian.clone()], "ndj?")
                .unwrap()
                .text(),
            "тво,"
        );
        assert_eq!(
            switch_keyboard_layout_text(&[english.clone(), russian.clone()], "rfr&")
                .unwrap()
                .text(),
            "как?"
        );

        let to_english = switch_keyboard_layout_text(&[english, russian], "руддщ")
            .expect("Russian physical spelling should map to English");
        assert_eq!(to_english.text(), "hello");
        assert_eq!(to_english.target_language().as_str(), "en");

        let english = language_pack_from_entries(
            LanguageId::try_new("en").unwrap(),
            vec![DictionaryEntry::try_new("hello", 1).unwrap()],
        )
        .unwrap();
        let russian = language_pack_from_entries(
            LanguageId::try_new("ru").unwrap(),
            vec![DictionaryEntry::try_new("как", 1).unwrap()],
        )
        .unwrap();
        let to_english_question = switch_keyboard_layout_text(&[english, russian], "как?")
            .expect("Russian shifted question mark key should map back to Latin shifted key");
        assert_eq!(to_english_question.text(), "rfr&");
        assert_eq!(to_english_question.target_language().as_str(), "en");
    }

    #[test]
    fn mixed_layout_switch_flips_each_symbol_and_uses_the_majority_target_language() {
        let english = language_pack_from_entries(
            LanguageId::try_new("en").unwrap(),
            vec![DictionaryEntry::try_new("hello", 1).unwrap()],
        )
        .unwrap();
        let russian = language_pack_from_entries(
            LanguageId::try_new("ru").unwrap(),
            vec![DictionaryEntry::try_new("привет", 1).unwrap()],
        )
        .unwrap();

        let switched = switch_keyboard_layout_text(&[english.clone(), russian.clone()], "tujю")
            .expect("mixed physical spelling with a majority target must be switchable");
        assert_eq!(switched.text(), "его.");
        assert_eq!(switched.target_language().as_str(), "ru");

        assert_eq!(
            switch_keyboard_layout_text(&[english, russian], "tю"),
            None,
            "an exact target-language vote tie must stay fail-closed"
        );
    }

    #[test]
    fn punctuation_only_layout_switch_uses_an_explicit_target_language() {
        let english = language_pack_from_entries(
            LanguageId::try_new("en").unwrap(),
            vec![DictionaryEntry::try_new("hello", 1).unwrap()],
        )
        .unwrap();
        let russian = language_pack_from_entries(
            LanguageId::try_new("ru").unwrap(),
            vec![DictionaryEntry::try_new("привет", 1).unwrap()],
        )
        .unwrap();
        let languages = [english, russian];

        assert_eq!(
            switch_keyboard_layout_text(&languages, "\"\""),
            None,
            "quote characters are physically ambiguous without a target layout"
        );

        let english_target = LanguageId::try_new("en").unwrap();
        let to_english =
            switch_keyboard_layout_text_to(&languages, "\"\"", &english_target).unwrap();
        assert_eq!(to_english.text(), "@@");
        assert_eq!(to_english.target_language(), &english_target);

        let russian_target = LanguageId::try_new("ru").unwrap();
        let to_russian = switch_keyboard_layout_text_to(&languages, "@@", &russian_target).unwrap();
        assert_eq!(to_russian.text(), "\"\"");
        assert_eq!(to_russian.target_language(), &russian_target);

        for (source, expected) in [
            ("~", "Ё"),
            ("@", "\""),
            ("#", "№"),
            ("$", ";"),
            ("^", ":"),
            ("{", "Х"),
            ("}", "Ъ"),
            (":", "Ж"),
            ("\"", "Э"),
            ("<", "Б"),
            (">", "Ю"),
            ("?", ","),
            ("&", "?"),
        ] {
            let switched =
                switch_keyboard_layout_text_to(&languages, source, &russian_target).unwrap();
            assert_eq!(switched.text(), expected, "source={source:?}");
        }

        for (source, expected) in [
            ("Ё", "~"),
            ("\"", "@"),
            ("№", "#"),
            (";", "$"),
            (":", "^"),
            ("Х", "{"),
            ("Ъ", "}"),
            ("Ж", ":"),
            ("Э", "\""),
            ("Б", "<"),
            ("Ю", ">"),
            (",", "?"),
            ("?", "&"),
        ] {
            let switched =
                switch_keyboard_layout_text_to(&languages, source, &english_target).unwrap();
            assert_eq!(switched.text(), expected, "source={source:?}");
        }
    }

    #[test]
    fn lower_case_pattern_does_not_inherit_canonical_capitalization() {
        assert_eq!(TextCasePattern::detect("сде").apply("Сделать"), "сделать");
        assert_eq!(TextCasePattern::detect("СДЕ").apply("сделать"), "СДЕЛАТЬ");
    }
}
