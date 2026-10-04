use super::{
    AppSettings, ClipboardEntryContent, ClipboardEntryKind, ClipboardEntryList,
    ClipboardHistoryLimit, ClipboardReorderPosition, Database, DatabaseError, SettingsError,
    UndoHotkey,
};

#[test]
fn file_database_uses_wal_normal_and_in_memory_keeps_memory_journal() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let memory = Database::open_in_memory().unwrap();
    let (memory_journal, _) = memory.storage_pragmas().unwrap();
    assert_eq!(memory_journal, "memory");

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sunswitcher-storage-{}-{nonce}.db",
        std::process::id()
    ));
    let database = Database::open(&path).unwrap();
    let (journal, synchronous) = database.storage_pragmas().unwrap();
    assert_eq!(journal, "wal");
    assert_eq!(synchronous, 1);
    drop(database);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[test]
fn default_settings_use_the_explicit_clipboard_history_limit() {
    let database = Database::open_in_memory().expect("fresh in-memory database should open");

    assert_eq!(database.schema_version().unwrap(), 6);
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
fn clipboard_text_history_deduplicates_orders_filters_and_pins() {
    let database = Database::open_in_memory().unwrap();

    let hello = database.record_clipboard_text("Hello", 10).unwrap();
    let world = database.record_clipboard_text("world", 20).unwrap();
    let hello_again = database.record_clipboard_text("Hello", 30).unwrap();

    assert_eq!(hello_again.id(), hello.id());
    assert_eq!(hello_again.kind(), ClipboardEntryKind::Text);
    assert_eq!(hello_again.copy_count(), 2);
    assert_eq!(hello_again.last_seen_at_ms(), 10);
    assert_eq!(
        hello_again.content(),
        &ClipboardEntryContent::Text("Hello".to_owned())
    );

    let current = database.load_clipboard_current("", 9).unwrap();
    assert_eq!(
        current.iter().map(|entry| entry.id()).collect::<Vec<_>>(),
        [hello.id(), world.id()]
    );
    assert_eq!(database.load_clipboard_current("HEL", 9).unwrap().len(), 1);

    let pinned = database.pin_clipboard_entry(world.id(), 40).unwrap();
    assert_eq!(pinned.pinned_at_ms(), Some(40));
    assert_eq!(
        database.load_clipboard_pinned("wor", 9).unwrap()[0].id(),
        world.id()
    );

    database.mark_clipboard_entry_used(world.id(), 50).unwrap();
    assert_eq!(
        database.load_clipboard_current("", 9).unwrap()[0].id(),
        world.id()
    );
    assert_eq!(
        database.load_clipboard_pinned("", 9).unwrap()[0].pinned_at_ms(),
        Some(40)
    );
}

#[test]
fn clipboard_current_entries_can_be_reordered_without_rewriting_entry_times() {
    let mut database = Database::open_in_memory().unwrap();

    let first = database.record_clipboard_text("first", 10).unwrap();
    let second = database.record_clipboard_text("second", 20).unwrap();
    let third = database.record_clipboard_text("third", 30).unwrap();

    database
        .reorder_clipboard_entry(
            ClipboardEntryList::Current,
            first.id(),
            third.id(),
            ClipboardReorderPosition::Before,
        )
        .unwrap();
    let current = database.load_clipboard_current("", 10).unwrap();
    assert_eq!(
        current.iter().map(|entry| entry.id()).collect::<Vec<_>>(),
        [first.id(), third.id(), second.id()]
    );
    assert_eq!(
        current
            .iter()
            .map(|entry| entry.last_seen_at_ms())
            .collect::<Vec<_>>(),
        [10, 30, 20]
    );

    database
        .reorder_clipboard_entry(
            ClipboardEntryList::Current,
            second.id(),
            first.id(),
            ClipboardReorderPosition::After,
        )
        .unwrap();
    let current = database.load_clipboard_current("", 10).unwrap();
    assert_eq!(
        current.iter().map(|entry| entry.id()).collect::<Vec<_>>(),
        [first.id(), second.id(), third.id()]
    );
    assert_eq!(
        current
            .iter()
            .map(|entry| entry.last_seen_at_ms())
            .collect::<Vec<_>>(),
        [10, 20, 30]
    );
}

#[test]
fn clipboard_pinned_entries_can_be_reordered_without_rewriting_pin_times() {
    let mut database = Database::open_in_memory().unwrap();

    let first = database.record_clipboard_text("first", 10).unwrap();
    let second = database.record_clipboard_text("second", 20).unwrap();
    let third = database.record_clipboard_text("third", 30).unwrap();
    database.pin_clipboard_entry(first.id(), 40).unwrap();
    database.pin_clipboard_entry(second.id(), 50).unwrap();
    database.pin_clipboard_entry(third.id(), 60).unwrap();

    database
        .reorder_clipboard_entry(
            ClipboardEntryList::Pinned,
            first.id(),
            third.id(),
            ClipboardReorderPosition::Before,
        )
        .unwrap();

    let pinned = database.load_clipboard_pinned("", 10).unwrap();
    assert_eq!(
        pinned.iter().map(|entry| entry.id()).collect::<Vec<_>>(),
        [first.id(), third.id(), second.id()]
    );
    assert_eq!(
        pinned
            .iter()
            .map(|entry| entry.pinned_at_ms())
            .collect::<Vec<_>>(),
        [Some(40), Some(60), Some(50)]
    );
}
#[test]
fn clipboard_image_history_is_loaded_with_text_entries() {
    let database = Database::open_in_memory().unwrap();
    let text = database.record_clipboard_text("hello", 10).unwrap();
    let image = database
        .record_clipboard_image("CF_DIB", &[1, 2, 3, 4], 20)
        .unwrap();

    assert_eq!(image.kind(), ClipboardEntryKind::Image);
    assert_eq!(
        image.content(),
        &ClipboardEntryContent::Image {
            format: "CF_DIB".to_owned(),
            data: vec![1, 2, 3, 4],
        }
    );
    assert_eq!(
        database
            .load_clipboard_current("image", 10)
            .unwrap()
            .iter()
            .map(|entry| entry.id())
            .collect::<Vec<_>>(),
        [image.id()]
    );
    assert_eq!(
        database
            .load_clipboard_current("", 10)
            .unwrap()
            .iter()
            .map(|entry| entry.id())
            .collect::<Vec<_>>(),
        [image.id(), text.id()]
    );
}

#[test]
fn clipboard_prune_keeps_pinned_entries_and_recent_current_entries() {
    let database = Database::open_in_memory().unwrap();
    let old = database.record_clipboard_text("old", 10).unwrap();
    let pinned = database.record_clipboard_text("pinned", 20).unwrap();
    let recent = database.record_clipboard_text("recent", 30).unwrap();
    database.pin_clipboard_entry(pinned.id(), 40).unwrap();

    assert_eq!(
        database
            .prune_clipboard_history(ClipboardHistoryLimit::try_new(1).unwrap())
            .unwrap(),
        1
    );
    assert!(matches!(
        database.mark_clipboard_entry_used(old.id(), 50),
        Err(DatabaseError::ClipboardEntryNotFound(_))
    ));
    assert_eq!(
        database
            .load_clipboard_current("", 10)
            .unwrap()
            .iter()
            .map(|entry| entry.id())
            .collect::<Vec<_>>(),
        [recent.id(), pinned.id()]
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

#[test]
fn user_word_delete_is_normalized_and_idempotent() {
    let database = Database::open_in_memory().unwrap();
    database
        .record_user_word("QuantileEntryStrategy1", 1)
        .unwrap();
    database.record_user_word("OtherToken", 2).unwrap();

    assert!(database.delete_user_word("QUANTILEENTRYSTRATEGY1").unwrap());
    assert!(!database.delete_user_word("quantileentrystrategy1").unwrap());

    let lexicon = database.load_user_lexicon().unwrap();
    assert!(!lexicon.contains_normalized("quantileentrystrategy1"));
    assert!(lexicon.contains_normalized("othertoken"));
}
