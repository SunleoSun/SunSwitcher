use super::lexical_provider::weighted_damerau_cost;
use super::{CorrectionCandidateProvider, LexicalCorrectionProvider};
use crate::input::{Boundary, CompletedToken, InputBuffer, InputEvent, InputOutcome, PhysicalKey};
use crate::persistence::Database;

fn provider() -> LexicalCorrectionProvider {
    let database = Database::open_in_memory().unwrap();
    LexicalCorrectionProvider::try_new(
        database.load_enabled_language_packs().unwrap(),
        database.load_user_lexicon().unwrap(),
    )
    .unwrap()
}

fn completed_physical_token(text: &str) -> CompletedToken {
    let mut buffer = InputBuffer::new();
    for character in text.chars() {
        let key = PhysicalKey::from_layout_symbol(character);
        buffer.process(InputEvent::typed_character(character, key));
    }
    let InputOutcome::Completed(token) = buffer.process(InputEvent::character(' ')) else {
        panic!("expected physical token completion");
    };
    token
}

#[test]
fn repeated_key_deletions_are_cheaper_than_generic_deletions() {
    assert!(weighted_damerau_cost("ддля", "для") < weighted_damerau_cost("адля", "для"));
    assert!(weighted_damerau_cost("ддляя", "для") < 1.0);
}

#[test]
fn adjacent_transposition_is_a_first_class_typo() {
    assert_eq!(weighted_damerau_cost("дял", "для"), 0.75);
    assert_eq!(weighted_damerau_cost("hlelo", "hello"), 0.75);
}

#[test]
fn valid_word_in_any_language_blocks_cross_language_overcorrection() {
    let provider = provider();
    for word in ["для", "hello", "мир", "world"] {
        let token = CompletedToken::new(word, Boundary::Character(' '));
        assert!(
            provider.candidates(&token).is_empty(),
            "valid word {word:?}"
        );
    }
}

#[test]
fn literal_punctuation_suffix_is_preserved_while_typo_core_is_corrected() {
    let provider = provider();
    let typo = completed_physical_token("hlelo,");
    assert!(
        provider
            .candidates(&typo)
            .iter()
            .any(|candidate| candidate.replacement().as_str() == "hello,")
    );

    let valid = completed_physical_token("hello,");
    assert!(provider.candidates(&valid).is_empty());
}

#[test]
fn punctuation_created_by_layout_transform_is_not_counted_as_a_typo() {
    let provider = provider();
    for (observed, expected) in [("руддщб", "hello,"), ("рдудщб", "hello,")] {
        let token = completed_physical_token(observed);
        assert!(
            provider
                .candidates(&token)
                .iter()
                .any(|candidate| candidate.replacement().as_str() == expected),
            "{observed:?}"
        );
    }
}

#[test]
fn provider_preserves_simple_case_pattern() {
    let provider = provider();
    let title = CompletedToken::new("Lkz", Boundary::Character(' '));
    let upper = CompletedToken::new("РУДДЩ", Boundary::Character(' '));

    assert!(
        provider
            .candidates(&title)
            .iter()
            .any(|candidate| candidate.replacement().as_str() == "Для")
    );
    assert!(
        provider
            .candidates(&upper)
            .iter()
            .any(|candidate| candidate.replacement().as_str() == "HELLO")
    );
}
