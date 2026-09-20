use super::{DictionaryEntry, KeyboardLayoutMap, LanguageId, LanguagePack};

const LATIN_QWERTY: &str = "`qwertyuiop[]asdfghjkl;'zxcvbnm,.";
const RUSSIAN_JCUKEN: &str = "ёйцукенгшщзхъфывапролджэячсмитьбю";

pub fn english_language_pack() -> LanguagePack {
    let entries = [
        ("the", 1000),
        ("and", 990),
        ("this", 970),
        ("that", 960),
        ("hello", 950),
        ("world", 940),
        ("for", 930),
        ("with", 920),
        ("yes", 900),
        ("no", 890),
        ("if", 880),
        ("can", 870),
        ("need", 860),
        ("text", 850),
        ("word", 840),
        ("test", 830),
        ("language", 820),
        ("work", 810),
        ("window", 800),
        ("program", 790),
        ("correction", 780),
        ("error", 770),
        ("errors", 760),
        ("english", 750),
        ("russian", 740),
        ("now", 730),
        ("later", 720),
        ("example", 710),
        ("keyboard", 700),
        ("sentence", 690),
    ]
    .into_iter()
    .map(|(word, frequency)| DictionaryEntry::try_new(word, frequency).unwrap())
    .collect();

    let russian_to_latin =
        KeyboardLayoutMap::from_aligned(RUSSIAN_JCUKEN, LATIN_QWERTY, 0.15).unwrap();

    LanguagePack::try_new(
        LanguageId::try_new("en").unwrap(),
        entries,
        vec![russian_to_latin],
    )
    .unwrap()
}
