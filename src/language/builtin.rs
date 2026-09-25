use std::sync::OnceLock;

use super::{LanguageId, LanguagePack, LanguagePackError, language_pack_from_fst_bytes};

static ENGLISH: OnceLock<Result<LanguagePack, LanguagePackError>> = OnceLock::new();
static RUSSIAN: OnceLock<Result<LanguagePack, LanguagePackError>> = OnceLock::new();

const ENGLISH_FST: &[u8] = include_bytes!("../../assets/dictionaries/en.fst");
const ENGLISH_CORRECTION_FST: &[u8] = include_bytes!("../../assets/dictionaries/en-correction.fst");
const RUSSIAN_FST: &[u8] = include_bytes!("../../assets/dictionaries/ru.fst");
const RUSSIAN_CORRECTION_FST: &[u8] = include_bytes!("../../assets/dictionaries/ru-correction.fst");

pub fn builtin_language_pack(id: &LanguageId) -> Result<Option<LanguagePack>, LanguagePackError> {
    let cached = match id.as_str() {
        "en" => ENGLISH.get_or_init(|| {
            language_pack_from_fst_bytes(
                id.clone(),
                ENGLISH_FST.to_vec(),
                ENGLISH_CORRECTION_FST.to_vec(),
            )
        }),
        "ru" => RUSSIAN.get_or_init(|| {
            language_pack_from_fst_bytes(
                id.clone(),
                RUSSIAN_FST.to_vec(),
                RUSSIAN_CORRECTION_FST.to_vec(),
            )
        }),
        _ => return Ok(None),
    };
    cached.clone().map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_builtin_dictionaries_cover_inflected_forms_and_yo() {
        let russian = builtin_language_pack(&LanguageId::try_new("ru").unwrap())
            .unwrap()
            .unwrap();
        for word in [
            "дом",
            "дома",
            "дому",
            "домом",
            "домами",
            "делаю",
            "делаешь",
            "красивому",
            "ёлка",
            "идёт",
            "приём",
            "эхо",
        ] {
            assert!(
                russian.contains_normalized(word),
                "missing Russian form: {word}"
            );
        }

        let english = builtin_language_pack(&LanguageId::try_new("en").unwrap())
            .unwrap()
            .unwrap();
        for word in [
            "work", "works", "worked", "working", "try", "tries", "tried", "trying", "child",
            "children",
        ] {
            assert!(
                english.contains_normalized(word),
                "missing English form: {word}"
            );
        }
    }

    #[test]
    fn builtin_completion_frequency_does_not_change_user_first_policy_contract() {
        let english = builtin_language_pack(&LanguageId::try_new("en").unwrap())
            .unwrap()
            .unwrap();
        let matches = english.prefix_matches("wor", 10);
        assert!(matches.iter().any(|entry| entry.word() == "work"));
    }
}
