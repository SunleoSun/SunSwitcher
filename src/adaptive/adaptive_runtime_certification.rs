use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use crate::correction::Confidence;
use crate::input::InputEvent;
use crate::persistence::Database;
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
fn certification_typed_technical_term_refreshes_live_snapshot_and_corrects_later_typo() {
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    let mut session = runtime.session();

    assert_eq!(
        type_token(&mut session, "QuantileEntryStrategy", 100),
        AdaptiveCorrectionDirective::Pass
    );
    runtime.flush().unwrap();
    assert!(
        runtime
            .snapshots()
            .load()
            .unwrap()
            .user_lexicon()
            .contains_normalized("quantileentrystrategy")
    );

    let correction = type_token(&mut session, "QuanntileEntrySrtategy", 200);
    let AdaptiveCorrectionDirective::Replace(action) = correction else {
        panic!("learned technical term must become a typo-correction candidate");
    };
    assert_eq!(action.replacement().as_str(), "QuantileEntryStrategy");
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
