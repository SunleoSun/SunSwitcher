use crate::correction::Confidence;
use crate::input::InputEvent;
use crate::persistence::Database;

use super::{AdaptiveCorrectionDirective, AdaptiveLexicalRuntime};

#[test]
fn plain_unknown_words_are_not_promoted_after_one_observation() {
    let runtime = AdaptiveLexicalRuntime::start(Database::open_in_memory().unwrap()).unwrap();
    let mut session = runtime.session(Confidence::try_new(0.80).unwrap());
    for character in "ordinaryunknown".chars() {
        assert_eq!(
            session
                .process(InputEvent::character(character), 10)
                .unwrap(),
            AdaptiveCorrectionDirective::Pass
        );
    }
    assert_eq!(
        session.process(InputEvent::character(' '), 10).unwrap(),
        AdaptiveCorrectionDirective::Pass
    );
    runtime.flush().unwrap();
    assert!(
        runtime
            .snapshots()
            .load()
            .unwrap()
            .user_lexicon()
            .is_empty()
    );
}

#[test]
fn system_dictionary_words_do_not_duplicate_into_user_vocabulary() {
    let runtime = AdaptiveLexicalRuntime::start(Database::open_in_memory().unwrap()).unwrap();
    runtime.learning().observe_typed_token("hello", 10).unwrap();
    runtime.flush().unwrap();
    assert!(
        runtime
            .snapshots()
            .load()
            .unwrap()
            .user_lexicon()
            .is_empty()
    );
}

#[test]
fn text_learning_extracts_identifiers_but_ignores_numeric_password_like_fragments() {
    let runtime = AdaptiveLexicalRuntime::start(Database::open_in_memory().unwrap()).unwrap();
    runtime
        .learning()
        .observe_text("copied QuantileEntryStrategy and 12/.2#$", 50)
        .unwrap();
    runtime.flush().unwrap();
    let snapshot = runtime.snapshots().load().unwrap();
    assert!(
        snapshot
            .user_lexicon()
            .contains_normalized("quantileentrystrategy")
    );
    assert!(!snapshot.user_lexicon().contains_normalized("12"));
}
