use super::{DictionaryEntry, KeyboardLayoutMap, LanguageId, LanguagePack};

const LATIN_QWERTY: &str = "`qwertyuiop[]asdfghjkl;'zxcvbnm,.";
const RUSSIAN_JCUKEN: &str = "ёйцукенгшщзхъфывапролджэячсмитьбю";

pub fn russian_language_pack() -> LanguagePack {
    let entries = [
        ("для", 1000),
        ("что", 980),
        ("это", 970),
        ("как", 960),
        ("привет", 940),
        ("жизнь", 935),
        ("хорошо", 930),
        ("ёлка", 925),
        ("можно", 920),
        ("объект", 915),
        ("быстро", 905),
        ("люди", 895),
        ("эхо", 885),
        ("юг", 875),
        ("нужно", 910),
        ("если", 900),
        ("да", 890),
        ("нет", 880),
        ("мир", 860),
        ("текст", 850),
        ("слово", 840),
        ("тест", 830),
        ("язык", 820),
        ("работа", 810),
        ("окно", 800),
        ("программа", 790),
        ("исправление", 780),
        ("ошибка", 770),
        ("ошибки", 760),
        ("русский", 750),
        ("английский", 740),
        ("сейчас", 730),
        ("потом", 720),
        ("пример", 710),
        ("клавиатура", 700),
        ("предложение", 690),
    ]
    .into_iter()
    .map(|(word, frequency)| DictionaryEntry::try_new(word, frequency).unwrap())
    .collect();

    let latin_to_russian =
        KeyboardLayoutMap::from_aligned(LATIN_QWERTY, RUSSIAN_JCUKEN, 0.15).unwrap();

    LanguagePack::try_new(
        LanguageId::try_new("ru").unwrap(),
        entries,
        vec![latin_to_russian],
    )
    .unwrap()
}
