mod english;
mod language_pack;
mod russian;

pub use language_pack::{
    DictionaryEntry, KeyboardLayoutMap, LanguageId, LanguagePack, LanguagePackError, normalize_word,
};

pub fn language_pack_from_entries(
    id: LanguageId,
    entries: Vec<DictionaryEntry>,
) -> Result<LanguagePack, LanguagePackError> {
    let transforms = match id.as_str() {
        "en" => english::layout_transforms()?,
        "ru" => russian::layout_transforms()?,
        _ => Vec::new(),
    };
    LanguagePack::try_new(id, entries, transforms)
}
