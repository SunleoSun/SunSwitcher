use super::{KeyboardLayoutMap, LanguagePackError};

const LATIN_QWERTY: &str = "`qwertyuiop[]asdfghjkl;'zxcvbnm,./?&";
const RUSSIAN_JCUKEN: &str = "ёйцукенгшщзхъфывапролджэячсмитьбю.,?";

pub(super) fn layout_transforms() -> Result<Vec<KeyboardLayoutMap>, LanguagePackError> {
    let map = KeyboardLayoutMap::from_aligned(RUSSIAN_JCUKEN, LATIN_QWERTY, 0.15)?
        .with_text_mappings("\"№;:", "@#$^")?;
    Ok(vec![map])
}
// end english layout transforms
