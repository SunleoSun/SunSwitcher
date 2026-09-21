use crate::lexicon::UserTermProtection;

use super::{
    AppSettings, ClipboardHistoryLimit, Database, DatabaseError, SettingsError, UndoHotkey,
};

#[test]
fn default_settings_use_the_explicit_clipboard_history_limit() {
    let database = Database::open_in_memory().expect("fresh in-memory database should open");

    assert_eq!(database.schema_version().unwrap(), 4);
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
fn user_term_upsert_accumulates_usage_and_makes_protection_sticky() {
    let database = Database::open_in_memory().unwrap();
    database
        .record_user_term("QuantileEntryStrategy1", UserTermProtection::Protected, 200)
        .unwrap();
    let updated = database
        .record_user_term("quantileentrystrategy1", UserTermProtection::Normal, 150)
        .unwrap();

    assert_eq!(updated.term(), "QuantileEntryStrategy1");
    assert_eq!(updated.use_count(), 2);
    assert_eq!(updated.last_used_at_ms(), 200);
    assert_eq!(updated.protection(), UserTermProtection::Protected);
}

#[test]
fn correction_undo_requires_valid_text_and_is_single_use() {
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

    let protected = database.commit_correction_undo(event, 20).unwrap();
    assert_eq!(protected.term(), "QuantileEntryStrategy1");
    assert_eq!(protected.protection(), UserTermProtection::Protected);
    assert!(matches!(
        database.prepare_correction_undo(event),
        Err(DatabaseError::CorrectionEventAlreadyUndone(_))
    ));
}
