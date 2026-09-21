use super::{
    AppSettings, ClipboardHistoryLimit, Database, DatabaseError, SettingsError, UndoHotkey,
};

#[test]
fn default_settings_use_the_explicit_clipboard_history_limit() {
    let database = Database::open_in_memory().expect("fresh in-memory database should open");

    assert_eq!(database.schema_version().unwrap(), 1);
    assert_eq!(
        database.settings().unwrap().clipboard_history_limit(),
        ClipboardHistoryLimit::DEFAULT
    );
    assert_eq!(ClipboardHistoryLimit::DEFAULT.get(), 1000);
    assert_eq!(
        database.settings().unwrap().undo_hotkey(),
        UndoHotkey::Pause
    );
    assert_eq!(UndoHotkey::DEFAULT, UndoHotkey::Pause);
}

#[test]
fn clipboard_history_limit_is_typed_and_round_trips_through_sqlite() {
    assert_eq!(
        ClipboardHistoryLimit::try_new(0),
        Err(SettingsError::ClipboardHistoryLimitMustBePositive)
    );

    let database = Database::open_in_memory().unwrap();
    let limit = ClipboardHistoryLimit::try_new(250).unwrap();
    database
        .save_settings(AppSettings::new(limit, UndoHotkey::Pause))
        .unwrap();

    assert_eq!(
        database.settings().unwrap().clipboard_history_limit(),
        limit
    );
}

#[test]
fn user_word_upsert_accumulates_usage_without_rewriting_canonical_spelling() {
    let database = Database::open_in_memory().unwrap();
    database
        .record_user_word("QuantileEntryStrategy1", 200)
        .unwrap();
    let older = database
        .record_user_word("quantileentrystrategy1", 150)
        .unwrap();
    assert_eq!(older.term(), "QuantileEntryStrategy1");
    assert_eq!(older.use_count(), 2);
    assert_eq!(older.last_used_at_ms(), 200);

    let newer = database
        .record_user_word("quantileentrystrategy1", 250)
        .unwrap();
    assert_eq!(newer.term(), "QuantileEntryStrategy1");
    assert_eq!(newer.use_count(), 3);
    assert_eq!(newer.last_used_at_ms(), 250);
}

#[test]
fn correction_undo_requires_valid_text_is_single_use_and_adds_a_user_word() {
    let mut database = Database::open_in_memory().unwrap();
    assert!(matches!(
        database.record_correction_event("", "fixed", 1),
        Err(DatabaseError::InvalidCorrectionText)
    ));

    let event = database
        .record_correction_event("QuantileEntryStrategy1", "QuantileEntryStrategy", 10)
        .unwrap();
    let plan = database.prepare_correction_undo(event).unwrap();
    assert_eq!(plan.original_text(), "QuantileEntryStrategy1");
    assert_eq!(plan.replacement_text(), "QuantileEntryStrategy");

    let learned = database.commit_correction_undo(event, 20).unwrap();
    assert_eq!(learned.term(), "QuantileEntryStrategy1");
    assert_eq!(learned.use_count(), 1);
    assert!(matches!(
        database.prepare_correction_undo(event),
        Err(DatabaseError::CorrectionEventAlreadyUndone(_))
    ));
}
