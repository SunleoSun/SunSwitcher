use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use crate::completion::{CompletionApplyOutcome, CompletionCommand, CompletionCommandResult};
use crate::correction::Confidence;
use crate::input::{Boundary, InputEvent, PhysicalKey};
use crate::persistence::{Database, ForgetWordOutcome, IgnoreWordOutcome};
use crate::replacement::{ReplacementOutcome, UndoOutcome};

use super::{AdaptiveCorrectionDirective, AdaptiveLexicalRuntime};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDatabasePath(PathBuf);

impl TempDatabasePath {
    fn new(label: &str) -> Self {
        let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "sunswitcher-adaptive-{label}-{}-{nanos}-{unique}.db",
            std::process::id()
        )))
    }

    fn as_path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDatabasePath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn type_token(
    session: &mut super::AdaptiveCorrectionSession,
    token: &str,
    at_ms: i64,
) -> AdaptiveCorrectionDirective {
    for character in token.chars() {
        assert_eq!(
            session
                .process(InputEvent::character(character), at_ms)
                .unwrap(),
            AdaptiveCorrectionDirective::Pass
        );
    }
    session.process(InputEvent::character(' '), at_ms).unwrap()
}

#[test]
fn certification_first_word_after_invalidation_is_corrected_without_leading_boundary() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    let mut session = runtime.session();

    assert_eq!(
        session.process(InputEvent::Invalidate, 50).unwrap(),
        AdaptiveCorrectionDirective::Pass
    );
    let correction = type_token(&mut session, "дял", 100);
    let AdaptiveCorrectionDirective::Replace(action) = correction else {
        panic!("first fresh token after invalidation must still reach lexical correction");
    };
    assert_eq!(action.replacement().as_str(), "для");
}

#[test]
fn certification_layout_switch_span_keeps_punctuation_until_whitespace() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    let mut session = runtime.session();

    for character in "ndj".chars() {
        assert_eq!(
            session
                .process(InputEvent::character(character), 100)
                .unwrap(),
            AdaptiveCorrectionDirective::Pass
        );
    }
    assert_eq!(
        session
            .process(InputEvent::typed_character('.', PhysicalKey::Period), 100,)
            .unwrap(),
        AdaptiveCorrectionDirective::Pass
    );
    assert_eq!(session.current_layout_switch_span(), "ndj.");

    session.tracked_layout_switch_applied("твою");
    assert_eq!(session.current_layout_switch_span(), "твою");
    assert_eq!(
        session.process(InputEvent::character(' '), 101).unwrap(),
        AdaptiveCorrectionDirective::Pass
    );
    assert_eq!(session.current_layout_switch_span(), "твою ");

    session.tracked_layout_switch_applied("ndj. ");
    assert_eq!(session.current_layout_switch_span(), "ndj. ");
}

#[test]
fn certification_one_letter_wrong_layout_words_are_replaced() {
    for (observed, expected) in [("z", "я"), ("b", "и")] {
        let runtime = AdaptiveLexicalRuntime::start(
            Database::open_in_memory().unwrap(),
            Confidence::try_new(0.80).unwrap(),
        )
        .unwrap();
        let mut session = runtime.session();

        let correction = type_token(&mut session, observed, 100);
        let AdaptiveCorrectionDirective::Replace(action) = correction else {
            panic!("{observed:?} must resolve to exact cross-layout word {expected:?}");
        };
        assert_eq!(action.replacement().as_str(), expected);
        assert_eq!(action.target_language().unwrap().as_str(), "ru");
    }
}

#[test]
fn certification_learned_single_letter_alias_yields_stronger_short_function_word() {
    let database = Database::open_in_memory().unwrap();
    for used_at_ms in 0..20 {
        database.record_user_word("b", used_at_ms).unwrap();
    }
    for used_at_ms in 100..284 {
        database.record_user_word("и", used_at_ms).unwrap();
    }
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
    let mut session = runtime.session();

    let correction = type_token(&mut session, "b", 100);
    let AdaptiveCorrectionDirective::Replace(action) = correction else {
        panic!("learned b must still resolve to stronger short function word и");
    };
    assert_eq!(action.replacement().as_str(), "и");
    assert_eq!(action.target_language().unwrap().as_str(), "ru");
}

#[test]
fn certification_cross_layout_learned_identifier_preserves_digits() {
    let database = Database::open_in_memory().unwrap();
    database.record_user_word("AA33", 10).unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
    let mut session = runtime.session();

    let correction = type_token(&mut session, "ФФ33", 100);
    let AdaptiveCorrectionDirective::Replace(action) = correction else {
        panic!("exact cross-layout learned identifier must be corrected");
    };
    assert_eq!(action.replacement().as_str(), "AA33");
    assert_eq!(action.target_language().unwrap().as_str(), "en");
}

#[test]
fn certification_first_wrong_layout_word_after_invalidation_is_corrected() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    let mut session = runtime.session();

    assert_eq!(
        session.process(InputEvent::Invalidate, 50).unwrap(),
        AdaptiveCorrectionDirective::Pass
    );
    let correction = type_token(&mut session, "lkz", 100);
    let AdaptiveCorrectionDirective::Replace(action) = correction else {
        panic!("first fresh wrong-layout token after invalidation must be corrected");
    };
    assert_eq!(action.replacement().as_str(), "для");
    assert_eq!(action.target_language().unwrap().as_str(), "ru");
}

#[test]
fn certification_explicit_user_word_deletion_refreshes_live_snapshot() {
    let database = Database::open_in_memory().unwrap();
    database.record_user_word("руддщ", 20).unwrap();
    database.record_text_history("hello руддщ", 21).unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();

    assert!(
        runtime
            .snapshots()
            .load()
            .unwrap()
            .user_lexicon()
            .contains_normalized("руддщ")
    );
    assert!(
        runtime
            .completion_provider()
            .unwrap()
            .complete_sequence(&["hello"], 25, 10)
            .iter()
            .any(|candidate| candidate.text() == "руддщ")
    );

    runtime
        .learning()
        .forget_word("РУДДЩ")
        .unwrap()
        .wait()
        .unwrap();
    runtime.flush().unwrap();
    assert!(
        !runtime
            .snapshots()
            .load()
            .unwrap()
            .user_lexicon()
            .contains_normalized("руддщ")
    );

    let mut session = runtime.session();
    let correction = type_token(&mut session, "руддщ", 30);
    let AdaptiveCorrectionDirective::Replace(action) = correction else {
        panic!(
            "deleting the learned wrong-layout spelling must expose the system correction again"
        );
    };
    assert_eq!(action.replacement().as_str(), "hello");
    assert_eq!(action.target_language().unwrap().as_str(), "en");
    assert!(
        runtime
            .completion_provider()
            .unwrap()
            .complete_sequence(&["hello"], 40, 10)
            .iter()
            .all(|candidate| candidate.text() != "руддщ")
    );
}

#[test]
fn certification_ignored_system_word_yields_to_wrong_layout_with_physical_punctuation() {
    let mut database = Database::open_in_memory().unwrap();
    assert_eq!(
        database.ignore_word("jr").unwrap(),
        IgnoreWordOutcome::AddedToIgnoreList
    );
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();

    let snapshot = runtime.snapshots().load().unwrap();
    assert!(snapshot.ignored_words().contains_normalized("jr"));
    assert!(!snapshot.contains_normalized("jr"));

    let mut plain = runtime.session();
    let AdaptiveCorrectionDirective::Replace(action) = type_token(&mut plain, "jr", 50) else {
        panic!("ignored English exact word must allow its Russian wrong-layout interpretation");
    };
    assert_eq!(action.replacement().as_str(), "ок");
    assert_eq!(action.target_language().unwrap().as_str(), "ru");

    let mut punctuated = runtime.session();
    assert_eq!(
        punctuated.process(InputEvent::character('j'), 60).unwrap(),
        AdaptiveCorrectionDirective::Pass
    );
    assert_eq!(
        punctuated.process(InputEvent::character('r'), 60).unwrap(),
        AdaptiveCorrectionDirective::Pass
    );
    assert_eq!(
        punctuated
            .process(InputEvent::typed_character('?', PhysicalKey::Slash), 60)
            .unwrap(),
        AdaptiveCorrectionDirective::Pass
    );
    let AdaptiveCorrectionDirective::Replace(action) =
        punctuated.process(InputEvent::character(' '), 60).unwrap()
    else {
        panic!("physical Shift+/ must remain in the wrong-layout span until boundary");
    };
    assert_eq!(action.replacement().as_str(), "ок,");
    assert_eq!(action.target_language().unwrap().as_str(), "ru");

    assert!(
        runtime
            .completion_provider()
            .unwrap()
            .complete("j", 20)
            .iter()
            .all(|candidate| candidate.text() != "jr")
    );
}

#[test]
fn certification_tracked_ignore_uses_the_lexical_core_before_deferred_punctuation() {
    let database = Database::open_in_memory().unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
    let mut session = runtime.session();

    assert_eq!(
        session.process(InputEvent::character('j'), 80).unwrap(),
        AdaptiveCorrectionDirective::Pass
    );
    assert_eq!(
        session.process(InputEvent::character('r'), 81).unwrap(),
        AdaptiveCorrectionDirective::Pass
    );
    let punctuation = char::from_u32(63).unwrap();
    assert_eq!(
        session
            .process(
                InputEvent::typed_character(punctuation, PhysicalKey::Slash),
                82
            )
            .unwrap(),
        AdaptiveCorrectionDirective::Pass
    );

    let term = session
        .current_lexical_term_for_hotkey()
        .unwrap()
        .expect("tracked lexical term");
    assert_eq!(term, "jr");
    assert_eq!(
        runtime
            .learning()
            .ignore_word(term)
            .unwrap()
            .wait()
            .unwrap(),
        IgnoreWordOutcome::AddedToIgnoreList
    );

    let AdaptiveCorrectionDirective::ReplaceLivePrefix(action) = session
        .recheck_current_layout_after_lexical_override(
            83,
            crate::correction::CorrectionFeaturePolicy::ALL,
        )
        .unwrap()
    else {
        panic!("tracked ignore must expose the physical-layout replacement");
    };
    assert_eq!(action.delete_previous_chars(), 3);
    assert_eq!(action.replacement().as_str(), "ок,");
}

#[test]
fn certification_live_ignore_rechecks_the_current_word_and_selected_spelling() {
    let database = Database::open_in_memory().unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();

    let mut jr = runtime.session();
    assert_eq!(
        jr.process(InputEvent::character('j'), 100).unwrap(),
        AdaptiveCorrectionDirective::Pass
    );
    assert_eq!(
        jr.process(InputEvent::character('r'), 101).unwrap(),
        AdaptiveCorrectionDirective::Pass
    );
    assert_eq!(
        runtime
            .learning()
            .ignore_word("jr")
            .unwrap()
            .wait()
            .unwrap(),
        IgnoreWordOutcome::AddedToIgnoreList
    );
    let AdaptiveCorrectionDirective::ReplaceLivePrefix(action) = jr
        .recheck_current_layout_after_lexical_override(
            102,
            crate::correction::CorrectionFeaturePolicy::ALL,
        )
        .unwrap()
    else {
        panic!("confirmed ignore must immediately expose jr -> ок");
    };
    assert_eq!(action.delete_previous_chars(), 2);
    assert_eq!(action.replacement().as_str(), "ок");
    assert_eq!(action.target_language().unwrap().as_str(), "ru");

    let selected = runtime.session();
    let correction = selected
        .recheck_text_layout_after_lexical_override(
            "jr",
            crate::correction::CorrectionFeaturePolicy::ALL,
        )
        .unwrap()
        .expect("selected ignored spelling must expose a cross-layout correction");
    assert_eq!(correction.as_str(), "ок");
    assert_eq!(correction.target_language().unwrap().as_str(), "ru");

    let mut yt = runtime.session();
    assert_eq!(
        yt.process(InputEvent::character('y'), 110).unwrap(),
        AdaptiveCorrectionDirective::Pass
    );
    assert_eq!(
        yt.process(InputEvent::character('t'), 111).unwrap(),
        AdaptiveCorrectionDirective::Pass
    );
    assert_eq!(
        runtime
            .learning()
            .ignore_word("yt")
            .unwrap()
            .wait()
            .unwrap(),
        IgnoreWordOutcome::AddedToIgnoreList
    );
    let AdaptiveCorrectionDirective::ReplaceLivePrefix(action) = yt
        .recheck_current_layout_after_lexical_override(
            112,
            crate::correction::CorrectionFeaturePolicy::ALL,
        )
        .unwrap()
    else {
        panic!("confirmed ignore must immediately expose yt -> не");
    };
    assert_eq!(action.replacement().as_str(), "не");
    assert_eq!(action.target_language().unwrap().as_str(), "ru");
}

#[test]
fn certification_ignored_word_blocks_learning_until_pause_style_forget() {
    let mut database = Database::open_in_memory().unwrap();
    database.ignore_word("мурзаплекс").unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
    let mut session = runtime.session();

    assert_eq!(
        type_token(&mut session, "мурзаплекс", 100),
        AdaptiveCorrectionDirective::Pass
    );
    runtime.flush().unwrap();
    assert!(
        !runtime
            .snapshots()
            .load()
            .unwrap()
            .user_lexicon()
            .contains_normalized("мурзаплекс")
    );

    assert_eq!(
        runtime
            .learning()
            .forget_word("МУРЗАПЛЕКС")
            .unwrap()
            .wait()
            .unwrap(),
        ForgetWordOutcome::RemovedFromIgnoreList
    );
    assert_eq!(
        type_token(&mut session, "мурзаплекс", 200),
        AdaptiveCorrectionDirective::Pass
    );
    runtime.flush().unwrap();
    assert!(
        runtime
            .snapshots()
            .load()
            .unwrap()
            .user_lexicon()
            .contains_normalized("мурзаплекс")
    );
}

#[test]
fn certification_startup_does_not_reinterpret_user_words_from_correction_history() {
    let database = Database::open_in_memory().unwrap();
    database
        .record_correction_event("CustomToken", "OtherToken", 10)
        .unwrap();
    database.record_user_word("CustomToken", 20).unwrap();

    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
    assert!(
        runtime
            .snapshots()
            .load()
            .unwrap()
            .user_lexicon()
            .contains_normalized("customtoken")
    );
}

#[test]
fn certification_typed_technical_term_refreshes_live_snapshot_and_corrects_later_typo() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    let mut session = runtime.session();

    assert_eq!(
        type_token(&mut session, "QuantileEntryStrategy1", 100),
        AdaptiveCorrectionDirective::Pass
    );
    runtime.flush().unwrap();
    assert!(
        runtime
            .snapshots()
            .load()
            .unwrap()
            .user_lexicon()
            .contains_normalized("quantileentrystrategy1")
    );

    let correction = type_token(&mut session, "QuanntileEntrySrtategy1", 200);
    let AdaptiveCorrectionDirective::Replace(action) = correction else {
        panic!("learned technical term must become a typo-correction candidate");
    };
    assert_eq!(action.replacement().as_str(), "QuantileEntryStrategy1");
}

#[test]
fn certification_typed_plain_word_becomes_a_typo_target_after_one_kept_use() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    let mut session = runtime.session();

    assert_eq!(
        type_token(&mut session, "мурзаплекс", 100),
        AdaptiveCorrectionDirective::Pass
    );
    runtime.flush().unwrap();

    let correction = type_token(&mut session, "мурзапелкс", 200);
    let AdaptiveCorrectionDirective::Replace(action) = correction else {
        panic!("a kept user word must become available to later typo correction");
    };
    assert_eq!(action.replacement().as_str(), "мурзаплекс");
}

#[test]
fn certification_copied_plain_word_becomes_a_typo_target_after_text_observation() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    runtime.learning().observe_text("мурзаплекс", 100).unwrap();
    runtime.flush().unwrap();
    let mut session = runtime.session();

    let correction = type_token(&mut session, "мурзапелкс", 200);
    let AdaptiveCorrectionDirective::Replace(action) = correction else {
        panic!("a copied user word must become available to later typo correction");
    };
    assert_eq!(action.replacement().as_str(), "мурзаплекс");
}

#[test]
fn certification_repeated_copied_user_word_preserves_per_occurrence_usage() {
    let database = Database::open_in_memory().unwrap();
    database
        .record_user_word("QuantileEntryStrategy", 10)
        .unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();

    runtime
        .learning()
        .observe_text("QuantileEntryStrategy QuantileEntryStrategy", 100)
        .unwrap();
    runtime.flush().unwrap();

    let snapshot = runtime.snapshots().load().unwrap();
    assert_eq!(
        snapshot
            .user_lexicon()
            .exact("quantileentrystrategy")
            .unwrap()
            .use_count(),
        3,
        "each kept clipboard occurrence must advance the canonical user-word usage count"
    );
}

#[test]
fn certification_copied_text_uses_words_learned_earlier_in_the_same_payload() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();

    runtime
        .learning()
        .observe_text("мурзаплекс мурзапелкс", 100)
        .unwrap();
    runtime.flush().unwrap();

    let snapshot = runtime.snapshots().load().unwrap();
    assert!(snapshot.user_lexicon().contains_normalized("мурзаплекс"));
    assert!(!snapshot.user_lexicon().contains_normalized("мурзапелкс"));
}

#[test]
fn certification_copied_text_does_not_learn_tokens_the_same_corrector_would_replace() {
    let database = Database::open_in_memory().unwrap();
    database
        .record_user_word("QuantileEntryStrategy", 10)
        .unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();

    runtime
        .learning()
        .observe_text("мурзаплекс дял QuanntileEntrySrtategy", 100)
        .unwrap();
    runtime.flush().unwrap();

    let snapshot = runtime.snapshots().load().unwrap();
    assert!(snapshot.user_lexicon().contains_normalized("мурзаплекс"));
    assert!(!snapshot.user_lexicon().contains_normalized("дял"));
    assert!(
        !snapshot
            .user_lexicon()
            .contains_normalized("quanntileentrysrtategy")
    );
    assert!(
        snapshot
            .user_lexicon()
            .contains_normalized("quantileentrystrategy")
    );
}

#[test]
fn certification_copied_sequence_is_canonicalized_and_repetition_beats_correction() {
    let database = Database::open_in_memory().unwrap();
    database
        .record_user_word("QuantileEntryStrategy", 10)
        .unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();

    runtime
        .learning()
        .observe_text("hello QuanntileEntrySrtategy", 100)
        .unwrap();
    runtime.learning().observe_text("hello world", 200).unwrap();
    runtime.learning().observe_text("hello world", 300).unwrap();
    runtime.flush().unwrap();

    let provider = runtime.completion_provider().unwrap();
    let completions = provider.complete_sequence(&["hello"], 300, 10);
    assert_eq!(completions[0].text(), "world");
    assert!(
        completions
            .iter()
            .any(|candidate| candidate.text() == "QuantileEntryStrategy")
    );
    assert!(
        !completions
            .iter()
            .any(|candidate| candidate.text() == "QuanntileEntrySrtategy")
    );
}

#[test]
fn certification_word_completion_combines_user_and_system_prefixes_case_insensitively() {
    let database = Database::open_in_memory().unwrap();
    database.record_user_word("PrototypeThing", 100).unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();

    let completions = runtime.completion_provider().unwrap().complete("Pro", 10);
    assert_eq!(completions[0].text(), "PrototypeThing");
    assert!(
        completions
            .iter()
            .any(|candidate| candidate.text() == "program")
    );
}

#[test]
fn certification_applied_completion_acceptance_increments_user_word_usage() {
    let database = Database::open_in_memory().unwrap();
    database.record_user_word("PrototypeThing", 100).unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
    let mut completion = runtime.completion_session();

    for (index, character) in "Pro".chars().enumerate() {
        completion
            .process_event(InputEvent::character(character), 110 + index as i64)
            .unwrap();
    }
    assert_eq!(completion.suggestions()[0].text(), "PrototypeThing");
    assert_eq!(
        completion.command(CompletionCommand::Accept).unwrap(),
        CompletionCommandResult::AcceptSuffix("totypeThing".to_owned())
    );
    completion
        .suffix_acceptance_outcome(CompletionApplyOutcome::Applied, 200)
        .unwrap();
    runtime.flush().unwrap();

    let snapshot = runtime.snapshots().load().unwrap();
    assert_eq!(
        snapshot
            .user_lexicon()
            .exact("prototypething")
            .unwrap()
            .use_count(),
        2
    );
}

#[test]
fn certification_applied_next_word_completion_persists_sequence_and_rehydrates_completion() {
    let path = TempDatabasePath::new("accepted-next-word-restart");
    {
        let database = Database::open(path.as_path()).unwrap();
        database.record_user_word("WordPrediction902", 10).unwrap();
        let runtime =
            AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
        let mut correction = runtime.session();
        let mut completion = runtime.completion_session();

        for (index, character) in "WordContext901 ".chars().enumerate() {
            let event = InputEvent::character(character);
            completion.process_event(event, 100 + index as i64).unwrap();
            assert_eq!(
                correction.process(event, 100 + index as i64).unwrap(),
                AdaptiveCorrectionDirective::Pass
            );
        }
        for (index, character) in "WordPred".chars().enumerate() {
            let event = InputEvent::character(character);
            completion.process_event(event, 200 + index as i64).unwrap();
            assert_eq!(
                correction.process(event, 200 + index as i64).unwrap(),
                AdaptiveCorrectionDirective::Pass
            );
        }
        assert_eq!(completion.suggestions()[0].text(), "WordPrediction902");
        assert_eq!(
            completion
                .command(CompletionCommand::AcceptNextWord)
                .unwrap(),
            CompletionCommandResult::AcceptWord("iction902 ".to_owned())
        );
        completion
            .word_acceptance_outcome(CompletionApplyOutcome::Applied, 300)
            .unwrap();
        runtime.flush().unwrap();
    }

    let raw = Connection::open(path.as_path()).unwrap();
    let accepted_use_count: i64 = raw
        .query_row(
            "SELECT use_count FROM user_words WHERE normalized_term = 'wordprediction902'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(accepted_use_count, 2);
    let accepted_sequence: (i64, i64) = raw
        .query_row(
            "SELECT count(*), COALESCE(MAX(use_count), 0) FROM text_history WHERE text = 'WordContext901 WordPrediction902'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(accepted_sequence, (1, 1));
    drop(raw);

    let database = Database::open(path.as_path()).unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
    let mut completion = runtime.completion_session();
    for (index, character) in "WordContext901 ".chars().enumerate() {
        completion
            .process_event(InputEvent::character(character), 400 + index as i64)
            .unwrap();
    }
    assert_eq!(
        completion.suggestions()[0].text(),
        "WordPrediction902",
        "accepted next-word completion must survive SQLite reopen and drive completion"
    );
}

#[test]
fn certification_applied_completion_acceptance_persists_sequence_and_rehydrates_completion() {
    let path = TempDatabasePath::new("accepted-completion-restart");
    {
        let database = Database::open(path.as_path()).unwrap();
        database
            .record_user_word("AcceptPrediction902", 10)
            .unwrap();
        let runtime =
            AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
        let mut correction = runtime.session();
        let mut completion = runtime.completion_session();

        for (index, character) in "AcceptContext901 ".chars().enumerate() {
            let event = InputEvent::character(character);
            completion.process_event(event, 100 + index as i64).unwrap();
            assert_eq!(
                correction.process(event, 100 + index as i64).unwrap(),
                AdaptiveCorrectionDirective::Pass
            );
        }
        for (index, character) in "AcceptPred".chars().enumerate() {
            let event = InputEvent::character(character);
            completion.process_event(event, 200 + index as i64).unwrap();
            assert_eq!(
                correction.process(event, 200 + index as i64).unwrap(),
                AdaptiveCorrectionDirective::Pass
            );
        }
        assert_eq!(completion.suggestions()[0].text(), "AcceptPrediction902");
        assert_eq!(
            completion.command(CompletionCommand::Accept).unwrap(),
            CompletionCommandResult::AcceptSuffix("iction902".to_owned())
        );
        completion
            .suffix_acceptance_outcome(CompletionApplyOutcome::Applied, 300)
            .unwrap();
        runtime.flush().unwrap();
    }

    let raw = Connection::open(path.as_path()).unwrap();
    let accepted_use_count: i64 = raw
        .query_row(
            "SELECT use_count FROM user_words WHERE normalized_term = 'acceptprediction902'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(accepted_use_count, 2);
    let accepted_sequence: (i64, i64) = raw
        .query_row(
            "SELECT count(*), COALESCE(MAX(use_count), 0) FROM text_history WHERE text = 'AcceptContext901 AcceptPrediction902'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(accepted_sequence, (1, 1));
    drop(raw);

    let database = Database::open(path.as_path()).unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
    let mut completion = runtime.completion_session();
    for (index, character) in "AcceptContext901 ".chars().enumerate() {
        completion
            .process_event(InputEvent::character(character), 400 + index as i64)
            .unwrap();
    }
    assert_eq!(
        completion.suggestions()[0].text(),
        "AcceptPrediction902",
        "accepted completion must survive SQLite reopen as the top context prediction"
    );
}

#[test]
fn certification_system_completion_acceptance_trains_sequence_without_duplicating_dictionary_word()
{
    let path = TempDatabasePath::new("system-completion-learning");
    {
        let database = Database::open(path.as_path()).unwrap();
        let runtime =
            AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
        let mut correction = runtime.session();
        let mut completion = runtime.completion_session();

        for (index, character) in "SystemContext901 ".chars().enumerate() {
            let event = InputEvent::character(character);
            completion.process_event(event, 100 + index as i64).unwrap();
            assert_eq!(
                correction.process(event, 100 + index as i64).unwrap(),
                AdaptiveCorrectionDirective::Pass
            );
        }
        for (index, character) in "progr".chars().enumerate() {
            let event = InputEvent::character(character);
            completion.process_event(event, 200 + index as i64).unwrap();
            assert_eq!(
                correction.process(event, 200 + index as i64).unwrap(),
                AdaptiveCorrectionDirective::Pass
            );
        }
        let program_index = completion
            .suggestions()
            .iter()
            .position(|candidate| candidate.text() == "program")
            .expect("built-in English completion must expose program for progr");
        for _ in 0..program_index {
            assert_eq!(
                completion.command(CompletionCommand::Next).unwrap(),
                CompletionCommandResult::Consumed
            );
        }
        assert_eq!(
            completion.suggestions()[completion.selected_index()].text(),
            "program"
        );
        assert_eq!(
            completion.command(CompletionCommand::Accept).unwrap(),
            CompletionCommandResult::AcceptSuffix("am".to_owned())
        );
        completion
            .suffix_acceptance_outcome(CompletionApplyOutcome::Applied, 300)
            .unwrap();
        runtime.flush().unwrap();
    }

    let raw = Connection::open(path.as_path()).unwrap();
    let user_word_count: i64 = raw
        .query_row(
            "SELECT count(*) FROM user_words WHERE normalized_term = 'program'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        user_word_count, 0,
        "accepting a system dictionary completion must not duplicate it into user_words"
    );
    let sequence_count: i64 = raw
        .query_row(
            "SELECT count(*) FROM text_history WHERE text = 'SystemContext901 program'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(sequence_count, 1);
    drop(raw);

    let database = Database::open(path.as_path()).unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
    let mut completion = runtime.completion_session();
    for (index, character) in "SystemContext901 ".chars().enumerate() {
        completion
            .process_event(InputEvent::character(character), 400 + index as i64)
            .unwrap();
    }
    assert_eq!(
        completion.suggestions()[0].text(),
        "program",
        "accepted system completion must rehydrate as the learned top context prediction"
    );
}

#[test]
fn certification_uncertain_completion_acceptance_persists_neither_usage_nor_sequence() {
    let path = TempDatabasePath::new("uncertain-completion-learning");
    {
        let database = Database::open(path.as_path()).unwrap();
        database
            .record_user_word("RejectPrediction902", 10)
            .unwrap();
        let runtime =
            AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
        let mut correction = runtime.session();
        let mut completion = runtime.completion_session();

        for (index, character) in "RejectContext901 ".chars().enumerate() {
            let event = InputEvent::character(character);
            completion.process_event(event, 100 + index as i64).unwrap();
            assert_eq!(
                correction.process(event, 100 + index as i64).unwrap(),
                AdaptiveCorrectionDirective::Pass
            );
        }
        for (index, character) in "RejectPred".chars().enumerate() {
            let event = InputEvent::character(character);
            completion.process_event(event, 200 + index as i64).unwrap();
            assert_eq!(
                correction.process(event, 200 + index as i64).unwrap(),
                AdaptiveCorrectionDirective::Pass
            );
        }
        assert_eq!(completion.suggestions()[0].text(), "RejectPrediction902");
        assert!(matches!(
            completion.command(CompletionCommand::Accept).unwrap(),
            CompletionCommandResult::AcceptSuffix(_)
        ));
        completion
            .suffix_acceptance_outcome(CompletionApplyOutcome::Uncertain, 300)
            .unwrap();
        runtime.flush().unwrap();
    }

    let raw = Connection::open(path.as_path()).unwrap();
    let use_count: i64 = raw
        .query_row(
            "SELECT use_count FROM user_words WHERE normalized_term = 'rejectprediction902'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(use_count, 1);
    let sequence_count: i64 = raw
        .query_row(
            "SELECT count(*) FROM text_history WHERE text = 'RejectContext901 RejectPrediction902'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(sequence_count, 0);
    drop(raw);

    let database = Database::open(path.as_path()).unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
    let mut completion = runtime.completion_session();
    for (index, character) in "RejectContext901 ".chars().enumerate() {
        completion
            .process_event(InputEvent::character(character), 400 + index as i64)
            .unwrap();
    }
    assert!(
        completion
            .suggestions()
            .iter()
            .all(|candidate| candidate.text() != "RejectPrediction902"),
        "uncertain completion insertion must not create persisted sequence evidence"
    );
}

#[test]
fn certification_live_completion_activates_at_three_characters_and_uses_sequence_context() {
    let database = Database::open_in_memory().unwrap();
    database.record_user_word("PrototypeThing", 100).unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
    runtime
        .learning()
        .observe_text("мне нужно сделать проект", 200)
        .unwrap();
    runtime.flush().unwrap();

    let mut completion = runtime.completion_session();
    completion
        .process_event(InputEvent::character('P'), 300)
        .unwrap();
    completion
        .process_event(InputEvent::character('r'), 301)
        .unwrap();
    assert!(!completion.is_active());
    completion
        .process_event(InputEvent::character('o'), 302)
        .unwrap();
    assert!(completion.is_active());
    assert!(
        completion
            .suggestions()
            .iter()
            .any(|candidate| candidate.text() == "PrototypeThing")
    );

    completion
        .process_event(InputEvent::Invalidate, 303)
        .unwrap();
    for character in "мне ".chars() {
        completion
            .process_event(InputEvent::character(character), 310)
            .unwrap();
    }
    for character in "нужно ".chars() {
        completion
            .process_event(InputEvent::character(character), 320)
            .unwrap();
    }
    assert!(completion.is_active());
    assert!(
        completion
            .suggestions()
            .iter()
            .any(|candidate| candidate.text() == "сделать проект")
    );
    for character in "сде".chars() {
        completion
            .process_event(InputEvent::character(character), 330)
            .unwrap();
    }

    assert!(completion.is_active());
    assert!(
        completion
            .suggestions()
            .iter()
            .any(|candidate| candidate.text() == "сделать проект")
    );
}

#[test]
fn certification_layout_switch_refreshes_completion_context() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    runtime.learning().observe_text("привет мир", 100).unwrap();
    runtime.flush().unwrap();

    let mut completion = runtime.completion_session();
    for character in "ghbdtn ".chars() {
        completion
            .process_event(InputEvent::character(character), 200)
            .unwrap();
    }
    assert!(
        !completion
            .suggestions()
            .iter()
            .any(|candidate| candidate.text() == "мир")
    );

    completion.layout_switch_applied("привет", 250).unwrap();
    assert!(completion.is_active());
    assert!(
        completion
            .suggestions()
            .iter()
            .any(|candidate| candidate.text() == "мир")
    );
}

#[test]
fn certification_layout_switch_replaces_an_active_completion_prefix() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    let mut completion = runtime.completion_session();
    for character in "ghbd".chars() {
        completion
            .process_event(InputEvent::character(character), 200)
            .unwrap();
    }

    completion.layout_switch_applied("прив", 250).unwrap();
    assert!(
        completion
            .suggestions()
            .iter()
            .any(|candidate| candidate.text() == "привет"),
        "a switch of the currently typed word must refresh from the replacement prefix"
    );
}

#[test]
fn certification_automatic_correction_refreshes_completion_context() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    runtime.learning().observe_text("hello world", 100).unwrap();
    runtime.flush().unwrap();

    let mut correction = runtime.session();
    let mut completion = runtime.completion_session();
    for character in "руддщ".chars() {
        let event = InputEvent::character(character);
        completion.process_event(event, 200).unwrap();
        assert_eq!(
            correction.process(event, 200).unwrap(),
            AdaptiveCorrectionDirective::Pass
        );
    }

    let boundary = InputEvent::character(' ');
    completion.process_event(boundary, 201).unwrap();
    let AdaptiveCorrectionDirective::Replace(action) = correction.process(boundary, 201).unwrap()
    else {
        panic!("wrong-layout hello must be corrected before completion refresh");
    };
    assert_eq!(action.replacement().as_str(), "hello");
    assert!(
        !completion
            .suggestions()
            .iter()
            .any(|candidate| candidate.text() == "world")
    );

    correction
        .replacement_outcome(ReplacementOutcome::Applied)
        .unwrap();
    let canonical = correction
        .take_resolved_completion_word()
        .expect("applied correction must expose a canonical completion word");
    completion.layout_switch_applied(&canonical, 202).unwrap();
    assert!(completion.is_active());
    assert!(
        completion
            .suggestions()
            .iter()
            .any(|candidate| candidate.text() == "world")
    );
}

#[test]
fn certification_non_space_correction_does_not_reopen_sequence_context() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    runtime.learning().observe_text("hello world", 100).unwrap();
    runtime.flush().unwrap();

    let mut correction = runtime.session();
    let mut completion = runtime.completion_session();
    for character in "руддщ".chars() {
        let event = InputEvent::character(character);
        completion.process_event(event, 200).unwrap();
        assert_eq!(
            correction.process(event, 200).unwrap(),
            AdaptiveCorrectionDirective::Pass
        );
    }

    let boundary = InputEvent::character('.');
    completion.process_event(boundary, 201).unwrap();
    let AdaptiveCorrectionDirective::Replace(action) = correction.process(boundary, 201).unwrap()
    else {
        panic!("wrong-layout hello before punctuation must be corrected");
    };
    assert_eq!(action.replacement().as_str(), "hello");
    correction
        .replacement_outcome(ReplacementOutcome::Applied)
        .unwrap();
    let canonical = correction
        .take_resolved_completion_word()
        .expect("applied correction must expose canonical completion word");
    completion.layout_switch_applied(&canonical, 202).unwrap();
    assert!(
        !completion
            .suggestions()
            .iter()
            .any(|candidate| candidate.text() == "world"),
        "punctuation must keep sequence context cleared after correction"
    );
}

#[test]
fn certification_live_prefix_reverse_switch_uses_hysteresis_state() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    runtime.learning().observe_text("data", 100).unwrap();
    runtime.flush().unwrap();

    let mut session = runtime.live_session();
    let mut directive = AdaptiveCorrectionDirective::Pass;
    for character in "dhtv".chars() {
        directive = session
            .process(InputEvent::character(character), 200)
            .unwrap();
    }
    let AdaptiveCorrectionDirective::ReplaceLivePrefix(action) = directive else {
        panic!("obvious wrong-layout English prefix must switch to Russian live prefix");
    };
    assert_eq!(action.replacement().as_str(), "врем");
    assert_eq!(action.target_language().unwrap().as_str(), "ru");
    session
        .live_prefix_replacement_outcome(ReplacementOutcome::Applied)
        .unwrap();
    assert_eq!(session.take_resolved_live_prefix().as_deref(), Some("врем"));

    for _ in 0.."врем".chars().count() {
        assert_eq!(
            session.process(InputEvent::Backspace, 250).unwrap(),
            AdaptiveCorrectionDirective::Pass
        );
    }

    let mut reverse = AdaptiveCorrectionDirective::Pass;
    for character in "вфеф".chars() {
        reverse = session
            .process(InputEvent::character(character), 300)
            .unwrap();
    }
    let AdaptiveCorrectionDirective::ReplaceLivePrefix(action) = reverse else {
        panic!("strong opposite evidence must reverse an active live-prefix layout switch");
    };
    assert_eq!(action.replacement().as_str(), "data");
    assert_eq!(action.target_language().unwrap().as_str(), "en");
}

#[test]
fn certification_sequence_prefix_and_word_acceptance_keep_selected_continuation() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    runtime
        .learning()
        .observe_text("project alpha beta", 100)
        .unwrap();
    runtime.flush().unwrap();

    let mut completion = runtime.completion_session();
    for (index, character) in "pro".chars().enumerate() {
        completion
            .process_event(InputEvent::character(character), 200 + index as i64)
            .unwrap();
    }
    let target_index = completion
        .suggestions()
        .iter()
        .position(|candidate| candidate.text() == "project alpha beta")
        .expect("stored phrase should be suggested directly from its first-word prefix");
    for _ in 0..target_index {
        assert_eq!(
            completion.command(CompletionCommand::Next).unwrap(),
            CompletionCommandResult::Consumed
        );
    }

    assert_eq!(
        completion
            .command(CompletionCommand::AcceptNextWord)
            .unwrap(),
        CompletionCommandResult::AcceptWord("ject ".to_owned())
    );

    runtime
        .learning()
        .observe_text("project alpha beta gamma", 250)
        .unwrap();
    for used_at_ms in 251..255 {
        runtime
            .learning()
            .observe_text("project alpha theta", used_at_ms)
            .unwrap();
    }
    runtime.flush().unwrap();

    completion
        .word_acceptance_outcome(CompletionApplyOutcome::Applied, 300)
        .unwrap();
    assert_eq!(completion.selected_index(), 0);
    assert_eq!(completion.suggestions()[0].text(), "alpha beta gamma");

    assert_eq!(
        completion
            .command(CompletionCommand::AcceptNextWord)
            .unwrap(),
        CompletionCommandResult::AcceptWord("alpha ".to_owned())
    );
    runtime
        .learning()
        .observe_text("project alpha beta gamma delta", 301)
        .unwrap();
    for used_at_ms in 302..306 {
        runtime
            .learning()
            .observe_text("project alpha beta omega", used_at_ms)
            .unwrap();
    }
    runtime.flush().unwrap();

    completion
        .word_acceptance_outcome(CompletionApplyOutcome::Applied, 310)
        .unwrap();
    assert_eq!(completion.selected_index(), 0);
    assert_eq!(completion.suggestions()[0].text(), "beta gamma delta");
}

#[test]
fn certification_delete_removes_selected_sequence_from_database_and_live_completion() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    runtime
        .learning()
        .observe_text("project alpha beta", 100)
        .unwrap();
    runtime.flush().unwrap();

    let mut completion = runtime.completion_session();
    for character in "pro".chars() {
        completion
            .process_event(InputEvent::character(character), 200)
            .unwrap();
    }
    let target_index = completion
        .suggestions()
        .iter()
        .position(|candidate| candidate.text() == "project alpha beta")
        .expect("learned sequence must be visible before deletion");
    for _ in 0..target_index {
        completion.command(CompletionCommand::Next).unwrap();
    }
    assert_eq!(
        completion
            .command(CompletionCommand::DeleteSelected)
            .unwrap(),
        CompletionCommandResult::Consumed
    );
    assert!(
        !completion
            .suggestions()
            .iter()
            .any(|candidate| candidate.text() == "project alpha beta")
    );
    runtime.flush().unwrap();

    let completions = runtime
        .completion_provider()
        .unwrap()
        .complete_sequence_for_prefix(&[], "pro", 300, 10);
    assert!(
        !completions
            .iter()
            .any(|candidate| candidate.text() == "project alpha beta")
    );
}

#[test]
fn certification_delete_hides_system_word_completion_without_removing_lexical_authority() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    let mut completion = runtime.completion_session();
    for character in "hel".chars() {
        completion
            .process_event(InputEvent::character(character), 100)
            .unwrap();
    }
    let target_index = completion
        .suggestions()
        .iter()
        .position(|candidate| candidate.text() == "hello")
        .expect("system word must be available before suppression");
    for _ in 0..target_index {
        completion.command(CompletionCommand::Next).unwrap();
    }
    assert_eq!(
        completion
            .command(CompletionCommand::DeleteSelected)
            .unwrap(),
        CompletionCommandResult::Consumed
    );
    runtime.flush().unwrap();

    assert!(
        !runtime
            .completion_provider()
            .unwrap()
            .complete("hel", 10)
            .iter()
            .any(|candidate| candidate.text() == "hello")
    );
    let snapshot = runtime.snapshots().load().unwrap();
    assert!(
        snapshot
            .languages()
            .iter()
            .any(|pack| pack.id().as_str() == "en" && pack.contains_normalized("hello"))
    );
}

#[test]
fn certification_repeated_typed_sequence_surfaces_multi_word_repetition() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    let mut session = runtime.session();

    for (index, token) in [
        "AA11", "AA22", "AA33", "AA11", "AA33", "AA11", "AA33", "AA33", "AA33", "AA33",
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            type_token(&mut session, token, 100 + index as i64),
            AdaptiveCorrectionDirective::Pass
        );
    }
    runtime.flush().unwrap();

    let completions = runtime
        .completion_provider()
        .unwrap()
        .complete_sequence_for_prefix(&["AA33", "AA33"], "AA3", 200, 10);
    assert_eq!(completions[0].text(), "AA33 AA33");

    let mut completion = runtime.completion_session();
    for character in "AA3".chars() {
        completion
            .process_event(InputEvent::character(character), 201)
            .unwrap();
    }
    assert_eq!(completion.suggestions()[0].text(), "AA33 AA33");
    assert_eq!(
        completion
            .command(CompletionCommand::AcceptNextWord)
            .unwrap(),
        CompletionCommandResult::AcceptWord("3 ".to_owned())
    );
    completion
        .word_acceptance_outcome(CompletionApplyOutcome::Applied, 202)
        .unwrap();
    assert!(
        completion.suggestions()[0]
            .text()
            .split_whitespace()
            .count()
            >= 2,
        "continuation collapsed too early: {:?}",
        completion.suggestions()[0].text()
    );

    assert_eq!(
        completion
            .command(CompletionCommand::AcceptNextWord)
            .unwrap(),
        CompletionCommandResult::AcceptWord("AA33 ".to_owned())
    );
    completion
        .word_acceptance_outcome(CompletionApplyOutcome::Applied, 203)
        .unwrap();
    assert!(
        completion.suggestions()[0]
            .text()
            .split_whitespace()
            .count()
            >= 2,
        "second Alt+Right collapsed to one word: {:?}",
        completion.suggestions()[0].text()
    );
}

#[test]
fn certification_typed_words_feed_rolling_sequence_history() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    let mut session = runtime.session();

    assert_eq!(
        type_token(&mut session, "hello", 100),
        AdaptiveCorrectionDirective::Pass
    );
    assert_eq!(
        type_token(&mut session, "world", 200),
        AdaptiveCorrectionDirective::Pass
    );
    assert_eq!(
        type_token(&mut session, "again", 300),
        AdaptiveCorrectionDirective::Pass
    );
    runtime.flush().unwrap();

    let completions = runtime
        .completion_provider()
        .unwrap()
        .complete_sequence(&["hello"], 400, 10);
    assert!(
        completions
            .iter()
            .any(|candidate| candidate.text() == "world again")
    );
}

#[test]
fn certification_alphanumeric_user_words_complete_from_alpha_prefix() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    let mut session = runtime.session();
    for (index, token) in ["sun1", "sun2", "sun1", "sun2", "sun3", "sun4"]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            type_token(&mut session, token, 100 + index as i64),
            AdaptiveCorrectionDirective::Pass,
            "unexpected correction for {token}"
        );
    }
    runtime.flush().unwrap();

    let words = runtime
        .completion_provider()
        .unwrap()
        .complete("sun", 10)
        .into_iter()
        .map(|candidate| candidate.text().to_owned())
        .collect::<Vec<_>>();
    for expected in ["sun1", "sun2", "sun3", "sun4"] {
        assert!(
            words.iter().any(|candidate| candidate == expected),
            "missing {expected}: {words:?}"
        );
    }

    let mut completion = runtime.completion_session();
    for character in "sun1 ".chars() {
        completion
            .process_event(InputEvent::character(character), 300)
            .unwrap();
    }
    completion
        .process_event(InputEvent::Boundary(Boundary::Enter), 301)
        .unwrap();
    assert!(!completion.is_active());
    for character in "sun".chars() {
        completion
            .process_event(InputEvent::character(character), 302)
            .unwrap();
    }
    let suggestions = completion
        .suggestions()
        .iter()
        .map(|candidate| candidate.text().to_owned())
        .collect::<Vec<_>>();
    for expected in ["sun1", "sun2", "sun3", "sun4"] {
        assert!(
            suggestions.iter().any(|candidate| candidate == expected),
            "new-line prefix must offer {expected}: {suggestions:?}"
        );
    }
}

#[test]
fn certification_typed_learning_persists_words_and_sequence_and_rehydrates_completion() {
    let path = TempDatabasePath::new("typed-learning-restart");
    {
        let database = Database::open(path.as_path()).unwrap();
        let runtime =
            AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
        let mut session = runtime.session();

        assert_eq!(
            type_token(&mut session, "TypedContext901", 100),
            AdaptiveCorrectionDirective::Pass
        );
        assert_eq!(
            type_token(&mut session, "TypedPrediction902", 200),
            AdaptiveCorrectionDirective::Pass
        );
        runtime.flush().unwrap();
    }

    let raw = Connection::open(path.as_path()).unwrap();
    for normalized in ["typedcontext901", "typedprediction902"] {
        let use_count: i64 = raw
            .query_row(
                "SELECT use_count FROM user_words WHERE normalized_term = ?1",
                [normalized],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            use_count, 1,
            "typed word {normalized} must persist exactly once"
        );
    }
    let typed_sequence: (i64, i64) = raw
        .query_row(
            "SELECT count(*), COALESCE(MAX(use_count), 0) FROM text_history WHERE text = 'TypedContext901 TypedPrediction902'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(typed_sequence, (1, 1));
    drop(raw);

    let database = Database::open(path.as_path()).unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
    let mut completion = runtime.completion_session();
    for (index, character) in "TypedContext901 ".chars().enumerate() {
        completion
            .process_event(InputEvent::character(character), 300 + index as i64)
            .unwrap();
    }
    assert_eq!(
        completion.suggestions()[0].text(),
        "TypedPrediction902",
        "physically typed sequence must survive SQLite reopen as the top context prediction"
    );
}

#[test]
fn certification_undo_replaces_corrected_word_in_typed_sequence_history() {
    let database = Database::open_in_memory().unwrap();
    database
        .record_user_word("QuantileEntryStrategy", 10)
        .unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
    let mut session = runtime.session();

    assert_eq!(
        type_token(&mut session, "hello", 50),
        AdaptiveCorrectionDirective::Pass
    );
    let correction = type_token(&mut session, "QuanntileEntrySrtategy", 100);
    let AdaptiveCorrectionDirective::Replace(action) = correction else {
        panic!("technical typo must be corrected before rolling Undo is tested");
    };
    assert_eq!(action.replacement().as_str(), "QuantileEntryStrategy");
    session
        .replacement_outcome(ReplacementOutcome::Applied)
        .unwrap();

    let undo = session
        .request_undo(110)
        .unwrap()
        .expect("applied correction must remain immediately undoable");
    assert_eq!(undo.replacement().as_str(), "QuanntileEntrySrtategy");
    session.undo_outcome(UndoOutcome::Applied).unwrap();
    runtime.flush().unwrap();

    let completions = runtime
        .completion_provider()
        .unwrap()
        .complete_sequence(&["hello"], 200, 10);
    assert!(
        completions
            .iter()
            .any(|candidate| candidate.text() == "QuanntileEntrySrtategy")
    );
    assert!(
        !completions
            .iter()
            .any(|candidate| candidate.text() == "QuantileEntryStrategy")
    );
}

#[test]
fn certification_aborted_correction_does_not_create_sequence_evidence() {
    let path = TempDatabasePath::new("aborted-sequence");
    {
        let database = Database::open(path.as_path()).unwrap();
        let runtime =
            AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
        let mut session = runtime.session();
        assert_eq!(
            type_token(&mut session, "world", 50),
            AdaptiveCorrectionDirective::Pass
        );
        let correction = type_token(&mut session, "hlelo", 100);
        assert!(matches!(
            correction,
            AdaptiveCorrectionDirective::Replace(_)
        ));
        session
            .replacement_outcome(ReplacementOutcome::Aborted)
            .unwrap();
        session.process(InputEvent::Invalidate, 110).unwrap();
        runtime.flush().unwrap();
    }

    let raw = Connection::open(path.as_path()).unwrap();
    let stale_sequence_count: i64 = raw
        .query_row(
            "SELECT count(*) FROM text_history WHERE text = 'world hlelo'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stale_sequence_count, 0);
}

#[test]
fn certification_corrected_deferred_punctuation_uses_canonical_sequence_word() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    let mut session = runtime.session();
    assert_eq!(
        type_token(&mut session, "world", 50),
        AdaptiveCorrectionDirective::Pass
    );
    for character in "hlelo".chars() {
        assert_eq!(
            session
                .process(InputEvent::character(character), 100)
                .unwrap(),
            AdaptiveCorrectionDirective::Pass
        );
    }
    assert_eq!(
        session
            .process(InputEvent::typed_character(',', PhysicalKey::Comma), 100,)
            .unwrap(),
        AdaptiveCorrectionDirective::Pass
    );
    let directive = session.process(InputEvent::character(' '), 100).unwrap();
    let AdaptiveCorrectionDirective::Replace(action) = directive else {
        panic!("punctuated typo must still correct");
    };
    assert_eq!(action.replacement().as_str(), "hello,");
    session
        .replacement_outcome(ReplacementOutcome::Applied)
        .unwrap();
    assert_eq!(
        session.process(InputEvent::character('x'), 110).unwrap(),
        AdaptiveCorrectionDirective::Pass
    );
    runtime.flush().unwrap();

    let completions = runtime
        .completion_provider()
        .unwrap()
        .complete_sequence(&["world"], 200, 10);
    assert!(
        completions
            .iter()
            .any(|candidate| candidate.text() == "hello")
    );
    assert!(
        !completions
            .iter()
            .any(|candidate| candidate.text() == "hello,")
    );
}

#[test]
fn certification_correction_history_records_only_an_applied_replacement() {
    let path = TempDatabasePath::new("replacement-outcome");
    {
        let database = Database::open(path.as_path()).unwrap();
        database
            .record_user_word("QuantileEntryStrategy", 10)
            .unwrap();
        let runtime =
            AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
        let mut session = runtime.session();
        assert!(matches!(
            type_token(&mut session, "QuanntileEntrySrtategy", 100),
            AdaptiveCorrectionDirective::Replace(_)
        ));
        session
            .replacement_outcome(ReplacementOutcome::Aborted)
            .unwrap();
        runtime.flush().unwrap();
    }

    let raw = Connection::open(path.as_path()).unwrap();
    let aborted_count: i64 = raw
        .query_row("SELECT count(*) FROM correction_events", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(aborted_count, 0);
    drop(raw);

    {
        let database = Database::open(path.as_path()).unwrap();
        let runtime =
            AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
        let mut session = runtime.session();
        assert!(matches!(
            type_token(&mut session, "QuanntileEntrySrtategy", 200),
            AdaptiveCorrectionDirective::Replace(_)
        ));
        session
            .replacement_outcome(ReplacementOutcome::Applied)
            .unwrap();
        runtime.flush().unwrap();
    }

    let raw = Connection::open(path.as_path()).unwrap();
    let applied: (i64, String, String) = raw
        .query_row(
            "SELECT count(*), observed_text, replacement_text FROM correction_events",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(applied.0, 1);
    assert_eq!(applied.1, "QuanntileEntrySrtategy");
    assert_eq!(applied.2, "QuantileEntryStrategy");
}

#[test]
fn certification_intervening_input_cancels_pending_correction_persistence() {
    let path = TempDatabasePath::new("stale-replacement-outcome");
    {
        let database = Database::open(path.as_path()).unwrap();
        database
            .record_user_word("QuantileEntryStrategy", 10)
            .unwrap();
        let runtime =
            AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
        let mut session = runtime.session();

        assert!(matches!(
            type_token(&mut session, "QuanntileEntrySrtategy", 100),
            AdaptiveCorrectionDirective::Replace(_)
        ));
        assert_eq!(
            session.process(InputEvent::Invalidate, 110).unwrap(),
            AdaptiveCorrectionDirective::Pass
        );
        session
            .replacement_outcome(ReplacementOutcome::Applied)
            .unwrap();
        runtime.flush().unwrap();
    }

    let raw = Connection::open(path.as_path()).unwrap();
    let count: i64 = raw
        .query_row("SELECT count(*) FROM correction_events", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn certification_immediate_hotkey_undo_restores_and_learns_the_original() {
    let database = Database::open_in_memory().unwrap();
    database
        .record_user_word("QuantileEntryStrategy", 10)
        .unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
    let mut session = runtime.session();

    let correction = type_token(&mut session, "QuanntileEntrySrtategy", 100);
    let AdaptiveCorrectionDirective::Replace(action) = correction else {
        panic!("technical typo must be corrected before Undo can be tested");
    };
    assert_eq!(action.replacement().as_str(), "QuantileEntryStrategy");
    session
        .replacement_outcome(ReplacementOutcome::Applied)
        .unwrap();

    let undo = session
        .request_undo(200)
        .unwrap()
        .expect("applied character-boundary correction must be immediately undoable");
    assert_eq!(undo.replacement().as_str(), "QuanntileEntrySrtategy");
    assert_eq!(undo.boundary(), crate::input::Boundary::Character(' '));
    session.undo_outcome(UndoOutcome::Applied).unwrap();
    runtime.flush().unwrap();

    let snapshot = runtime.snapshots().load().unwrap();
    let learned = snapshot
        .user_lexicon()
        .exact("quanntileentrysrtategy")
        .unwrap();
    assert_eq!(learned.term(), "QuanntileEntrySrtategy");
}

#[test]
fn certification_intervening_input_disarms_immediate_undo() {
    let database = Database::open_in_memory().unwrap();
    database
        .record_user_word("QuantileEntryStrategy", 10)
        .unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
    let mut session = runtime.session();

    assert!(matches!(
        type_token(&mut session, "QuanntileEntrySrtategy", 100),
        AdaptiveCorrectionDirective::Replace(_)
    ));
    session
        .replacement_outcome(ReplacementOutcome::Applied)
        .unwrap();
    runtime.flush().unwrap();

    assert_eq!(
        session.process(InputEvent::character('x'), 150).unwrap(),
        AdaptiveCorrectionDirective::Pass
    );
    assert!(session.request_undo(200).unwrap().is_none());
}

#[test]
fn certification_intervening_input_cancels_inflight_undo_commit() {
    let path = TempDatabasePath::new("stale-undo-outcome");
    {
        let database = Database::open(path.as_path()).unwrap();
        database
            .record_user_word("QuantileEntryStrategy", 10)
            .unwrap();
        let runtime =
            AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
        let mut session = runtime.session();

        assert!(matches!(
            type_token(&mut session, "QuanntileEntrySrtategy", 100),
            AdaptiveCorrectionDirective::Replace(_)
        ));
        session
            .replacement_outcome(ReplacementOutcome::Applied)
            .unwrap();
        assert!(session.request_undo(200).unwrap().is_some());
        assert_eq!(
            session.process(InputEvent::Invalidate, 210).unwrap(),
            AdaptiveCorrectionDirective::Pass
        );
        session.undo_outcome(UndoOutcome::Applied).unwrap();
        runtime.flush().unwrap();
    }

    let raw = Connection::open(path.as_path()).unwrap();
    let undone_at: Option<i64> = raw
        .query_row(
            "SELECT undone_at_ms FROM correction_events ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let learned_count: i64 = raw
        .query_row(
            "SELECT count(*) FROM user_words WHERE normalized_term = 'quanntileentrysrtategy'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(undone_at, None);
    assert_eq!(learned_count, 0);
}

#[test]
fn certification_nonexecuted_undo_can_be_retried_but_uncertain_undo_cannot() {
    let database = Database::open_in_memory().unwrap();
    database
        .record_user_word("QuantileEntryStrategy", 10)
        .unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
    let mut session = runtime.session();

    assert!(matches!(
        type_token(&mut session, "QuanntileEntrySrtategy", 100),
        AdaptiveCorrectionDirective::Replace(_)
    ));
    session
        .replacement_outcome(ReplacementOutcome::Applied)
        .unwrap();
    runtime.flush().unwrap();

    assert!(session.request_undo(200).unwrap().is_some());
    session.undo_outcome(UndoOutcome::NotExecuted).unwrap();
    assert!(session.request_undo(210).unwrap().is_some());
    session.undo_outcome(UndoOutcome::Uncertain).unwrap();
    assert!(session.request_undo(220).unwrap().is_none());
}

#[test]
fn certification_undo_commit_learns_original_and_refreshes_live_snapshot() {
    let database = Database::open_in_memory().unwrap();
    let event = database
        .record_correction_event("QuantileEntryStrategy1", "QuantileEntryStrategy", 100)
        .unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();

    let plan = runtime.learning().prepare_undo(event).unwrap();
    assert_eq!(plan.original_text(), "QuantileEntryStrategy1");
    assert_eq!(plan.replacement_text(), "QuantileEntryStrategy");
    runtime.learning().commit_undo(event, 200).unwrap();
    runtime.flush().unwrap();

    let snapshot = runtime.snapshots().load().unwrap();
    let learned = snapshot
        .user_lexicon()
        .exact("quantileentrystrategy1")
        .unwrap();
    assert_eq!(learned.term(), "QuantileEntryStrategy1");

    let completions = runtime
        .completion_provider()
        .unwrap()
        .complete("QuantileEntryStrategy", 10);
    assert!(
        completions
            .iter()
            .any(|candidate| candidate.text() == "QuantileEntryStrategy1")
    );
}

#[test]
fn certification_existing_user_word_usage_refreshes_recency_without_sql_on_correction_lookup() {
    let database = Database::open_in_memory().unwrap();
    database
        .record_user_word("QuantileEntryStrategy", 10)
        .unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
    let mut session = runtime.session();

    assert_eq!(
        type_token(&mut session, "QuantileEntryStrategy", 500),
        AdaptiveCorrectionDirective::Pass
    );
    runtime.flush().unwrap();
    let snapshot = runtime.snapshots().load().unwrap();
    let term = snapshot
        .user_lexicon()
        .exact("quantileentrystrategy")
        .unwrap();
    assert_eq!(term.last_used_at_ms(), 500);
    assert_eq!(term.use_count(), 2);
}
