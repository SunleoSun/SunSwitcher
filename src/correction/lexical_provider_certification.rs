use super::{Confidence, CorrectionDecision, CorrectionEngine, LexicalCorrectionProvider};
use crate::input::{Boundary, CompletedToken, InputBuffer, InputEvent, InputOutcome, PhysicalKey};
use crate::persistence::Database;

fn engine() -> CorrectionEngine<LexicalCorrectionProvider> {
    let database = Database::open_in_memory().unwrap();
    let languages = database.load_enabled_language_packs().unwrap();
    let user_lexicon = database.load_user_lexicon().unwrap();
    CorrectionEngine::new(
        LexicalCorrectionProvider::try_new(languages, user_lexicon).unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
}

fn replacement_for(input: &str) -> Option<String> {
    let token = CompletedToken::new(input, Boundary::Character(' '));
    replacement_for_token(token)
}

fn replacement_for_physical_input(input: &str) -> Option<String> {
    let mut buffer = InputBuffer::new();
    for character in input.chars() {
        let physical_key = PhysicalKey::from_layout_symbol(character);
        assert_eq!(
            buffer.process(InputEvent::typed_character(character, physical_key)),
            InputOutcome::Continue
        );
    }
    let InputOutcome::Completed(token) = buffer.process(InputEvent::character(' ')) else {
        panic!("expected token completion for {input:?}");
    };
    replacement_for_token(token)
}

fn replacement_for_token(token: CompletedToken) -> Option<String> {
    match engine().decide(&token) {
        CorrectionDecision::Keep => None,
        CorrectionDecision::Replace(replacement) => Some(replacement.as_str().to_owned()),
    }
}

#[test]
fn certification_russian_typo_classes_converge_to_dictionary_word() {
    for (observed, expected) in [
        ("дял", "для"),
        ("ддля", "для"),
        ("ддляя", "для"),
        ("привте", "привет"),
        ("привт", "привет"),
        ("привеет", "привет"),
    ] {
        assert_eq!(
            replacement_for(observed).as_deref(),
            Some(expected),
            "{observed:?}"
        );
    }
}

#[test]
fn certification_english_typo_classes_use_the_same_language_agnostic_algorithm() {
    for (observed, expected) in [("hlelo", "hello"), ("helllo", "hello"), ("helo", "hello")] {
        assert_eq!(
            replacement_for(observed).as_deref(),
            Some(expected),
            "{observed:?}"
        );
    }
}

#[test]
fn certification_wrong_layout_and_typo_can_be_corrected_in_one_candidate_path() {
    for (observed, expected) in [
        ("lkz", "для"),
        ("llkz", "для"),
        ("lzk", "для"),
        ("руддщ", "hello"),
        ("рдудщ", "hello"),
        ("рудддщ", "hello"),
    ] {
        assert_eq!(
            replacement_for(observed).as_deref(),
            Some(expected),
            "{observed:?}"
        );
    }
}

#[test]
fn certification_layout_ambiguous_punctuation_keys_cover_full_russian_keyboard_words() {
    for (observed, expected) in [
        (";bpym", "жизнь"),
        ("'[j", "эхо"),
        ("[jhjij", "хорошо"),
        ("j,]trn", "объект"),
        (",scnhj", "быстро"),
        ("k.lb", "люди"),
        ("`krf", "ёлка"),
    ] {
        assert_eq!(
            replacement_for_physical_input(observed).as_deref(),
            Some(expected),
            "{observed:?}"
        );
    }
}

#[test]
fn certification_shifted_layout_ambiguous_key_preserves_target_case() {
    assert_eq!(
        replacement_for_physical_input(":bpym").as_deref(),
        Some("Жизнь")
    );
    assert_eq!(
        replacement_for_physical_input("\"[j").as_deref(),
        Some("Эхо")
    );
}

#[test]
fn certification_literal_punctuation_remains_punctuation_for_valid_or_corrected_english() {
    assert_eq!(replacement_for_physical_input("hello,"), None);
    assert_eq!(replacement_for_physical_input("world."), None);
    assert_eq!(
        replacement_for_physical_input("hlelo,").as_deref(),
        Some("hello,")
    );
    assert_eq!(
        replacement_for_physical_input("hlelo.").as_deref(),
        Some("hello.")
    );
}

#[test]
fn certification_target_layout_punctuation_is_preserved_before_typo_scoring() {
    for (observed, expected) in [
        ("руддщб", "hello,"),
        ("рдудщб", "hello,"),
        ("руддщю", "hello."),
        ("рдудщю", "hello."),
        ("РУДДЩБ", "HELLO<"),
        ("РУДДЩЮ", "HELLO>"),
    ] {
        assert_eq!(
            replacement_for_physical_input(observed).as_deref(),
            Some(expected),
            "{observed:?}"
        );
    }
}

#[test]
fn certification_valid_bilingual_words_and_unknown_noise_fail_closed() {
    for observed in ["для", "привет", "hello", "world", "qzxv"] {
        assert_eq!(replacement_for(observed), None, "{observed:?}");
    }
}

#[test]
fn certification_valid_surface_forms_are_not_collapsed_to_seed_lemmas() {
    for observed in [
        "слову",
        "словом",
        "словах",
        "ошибке",
        "программы",
        "языков",
        "русскому",
        "людей",
        "words",
        "working",
        "languages",
        "examples",
    ] {
        assert_eq!(
            replacement_for(observed),
            None,
            "valid surface form {observed:?}"
        );
    }
}

#[test]
fn certification_typos_of_surface_forms_preserve_the_inflected_target() {
    for (observed, expected) in [("слвоу", "слову"), ("wrods", "words")] {
        assert_eq!(
            replacement_for(observed).as_deref(),
            Some(expected),
            "{observed:?}"
        );
    }
}

#[test]
fn certification_case_survives_cross_layout_correction() {
    assert_eq!(replacement_for("Lkz").as_deref(), Some("Для"));
    assert_eq!(replacement_for("РУДДЩ").as_deref(), Some("HELLO"));
}

#[test]
fn certification_user_lexicon_corrects_typos_and_keeps_exact_user_words() {
    let database = Database::open_in_memory().unwrap();
    database
        .record_user_word("QuantileEntryStrategy", 100)
        .unwrap();
    database
        .record_user_word("QuantileEntryStrategy1", 200)
        .unwrap();
    let engine = CorrectionEngine::new(
        LexicalCorrectionProvider::try_new(
            database.load_enabled_language_packs().unwrap(),
            database.load_user_lexicon().unwrap(),
        )
        .unwrap(),
        Confidence::try_new(0.80).unwrap(),
    );

    let typo = CompletedToken::new("QuanntileEntrySrtategy", Boundary::Character(' '));
    assert_eq!(
        match engine.decide(&typo) {
            CorrectionDecision::Replace(replacement) => Some(replacement.as_str().to_owned()),
            CorrectionDecision::Keep => None,
        }
        .as_deref(),
        Some("QuantileEntryStrategy")
    );

    let exact_user_word = CompletedToken::new("QuantileEntryStrategy1", Boundary::Character(' '));
    assert_eq!(engine.decide(&exact_user_word), CorrectionDecision::Keep);
}
