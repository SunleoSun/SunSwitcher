use super::{KeyboardLayoutMap, LanguagePackError};

const LATIN_QWERTY: &str = "`qwertyuiop[]asdfghjkl;'zxcvbnm,.";
const RUSSIAN_JCUKEN: &str = "ёйцукенгшщзхъфывапролджэячсмитьбю";

pub(super) fn layout_transforms() -> Result<Vec<KeyboardLayoutMap>, LanguagePackError> {
    Ok(vec![KeyboardLayoutMap::from_aligned(
        LATIN_QWERTY,
        RUSSIAN_JCUKEN,
        0.15,
    )?])
}
