use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use super::{AppSettings, ClipboardHistoryLimit, Database, DatabaseError, UndoHotkey};

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
            "sunswitcher-{label}-{}-{nanos}-{unique}.db",
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

#[test]
fn certification_fresh_database_has_only_the_required_application_tables() {
    let path = TempDatabasePath::new("schema");
    let database = Database::open(path.as_path()).unwrap();
    assert_eq!(database.schema_version().unwrap(), 6);
    drop(database);

    let raw = Connection::open(path.as_path()).unwrap();
    let mut statement = raw
        .prepare(
            "SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .unwrap();
    let tables: Vec<String> = statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    assert_eq!(
        tables,
        [
            "app_settings",
            "clipboard_entries",
            "clipboard_files",
            "clipboard_images",
            "clipboard_text",
            "completion_hidden_words",
            "correction_events",
            "languages",
            "text_history",
            "user_words",
        ]
    );
    assert!(!tables.iter().any(|table| table == "schema_migrations"));
}

#[test]
fn certification_user_words_are_language_neutral_in_canonical_schema() {
    let path = TempDatabasePath::new("language-neutral-user-terms");
    let database = Database::open(path.as_path()).unwrap();
    drop(database);

    let raw = Connection::open(path.as_path()).unwrap();
    let mut statement = raw.prepare("PRAGMA table_info(user_words)").unwrap();
    let columns: Vec<String> = statement
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!columns.iter().any(|column| column == "language_id"));
    assert!(!columns.iter().any(|column| column == "protected"));
}

#[test]
fn certification_unknown_stored_undo_hotkey_fails_closed() {
    let path = TempDatabasePath::new("invalid-undo-hotkey");
    let database = Database::open(path.as_path()).unwrap();
    drop(database);
    let raw = Connection::open(path.as_path()).unwrap();
    raw.execute(
        "UPDATE app_settings SET undo_hotkey = 'future_key' WHERE singleton = 1",
        [],
    )
    .unwrap();
    drop(raw);

    let reopened = Database::open(path.as_path()).unwrap();
    assert!(matches!(
        reopened.settings(),
        Err(DatabaseError::InvalidStoredUndoHotkey(value)) if value == "future_key"
    ));
}

#[test]
fn certification_dictionary_surface_rows_build_runtime_language_packs() {
    let database = Database::open_in_memory().unwrap();
    let packs = database.load_enabled_language_packs().unwrap();
    assert_eq!(packs.len(), 2);

    let russian = packs
        .iter()
        .find(|pack| pack.id().as_str() == "ru")
        .expect("Russian dictionary language must be enabled");
    for word in [
        "для",
        "жизнь",
        "домами",
        "делаешь",
        "красивому",
        "ёлка",
        "идёт",
        "приём",
    ] {
        assert!(
            russian.contains_normalized(word),
            "missing Russian form: {word}"
        );
    }
    assert_eq!(russian.transforms().len(), 1);

    let english = packs
        .iter()
        .find(|pack| pack.id().as_str() == "en")
        .expect("English dictionary language must be enabled");
    for word in [
        "hello", "world", "works", "worked", "working", "tries", "tried", "children",
    ] {
        assert!(
            english.contains_normalized(word),
            "missing English form: {word}"
        );
    }
    assert_eq!(english.transforms().len(), 1);
}

#[test]
fn certification_builtin_vocabulary_is_not_duplicated_in_sqlite() {
    let path = TempDatabasePath::new("builtin-dictionary-authority");
    {
        let database = Database::open(path.as_path()).unwrap();
        assert_eq!(database.schema_version().unwrap(), 6);
    }

    let raw = Connection::open(path.as_path()).unwrap();
    let exists: i64 = raw
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'dictionary_words'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(exists, 0);
}

#[test]
fn certification_unknown_enabled_language_fails_closed() {
    let path = TempDatabasePath::new("unknown-language");
    {
        let database = Database::open(path.as_path()).unwrap();
        assert_eq!(database.schema_version().unwrap(), 6);
    }

    let raw = Connection::open(path.as_path()).unwrap();
    raw.execute(
        "INSERT INTO languages (code, display_name, enabled) VALUES ('xx', 'Custom', 1)",
        [],
    )
    .unwrap();
    drop(raw);

    let database = Database::open(path.as_path()).unwrap();
    assert!(matches!(
        database.load_enabled_language_packs(),
        Err(DatabaseError::InvalidStoredLanguageCode(code)) if code == "xx"
    ));
}

#[test]
fn certification_user_words_survive_reopen_and_rebuild_ranked_runtime_snapshot() {
    let path = TempDatabasePath::new("user-terms");
    {
        let database = Database::open(path.as_path()).unwrap();
        database
            .record_user_word("QuantileEntryStrategy", 100)
            .unwrap();
        database
            .record_user_word("QuantileEntryStrategy1", 200)
            .unwrap();
        database
            .record_user_word("quantileentrystrategy1", 150)
            .unwrap();
    }

    let reopened = Database::open(path.as_path()).unwrap();
    let lexicon = reopened.load_user_lexicon().unwrap();
    let learned = lexicon.exact("quantileentrystrategy1").unwrap();
    assert_eq!(learned.term(), "QuantileEntryStrategy1");
    assert_eq!(learned.use_count(), 2);
    assert_eq!(learned.last_used_at_ms(), 200);
    assert_eq!(
        lexicon
            .prefix_matches("QuantileEntry", 2)
            .iter()
            .map(|entry| entry.term())
            .collect::<Vec<_>>(),
        ["QuantileEntryStrategy1", "QuantileEntryStrategy"]
    );
}

#[test]
fn certification_text_history_accumulates_repetition_and_survives_reopen() {
    let path = TempDatabasePath::new("text-history");
    {
        let database = Database::open(path.as_path()).unwrap();
        database.record_text_history("hello world", 100).unwrap();
        database.record_text_history("hello world", 200).unwrap();
    }

    let reopened = Database::open(path.as_path()).unwrap();
    let history = reopened.load_text_history().unwrap();
    let entry = history
        .entries()
        .iter()
        .find(|entry| entry.text() == "hello world")
        .unwrap();
    assert_eq!(entry.use_count(), 2);
    assert_eq!(entry.last_used_at_ms(), 200);
}

#[test]
fn certification_hidden_completion_word_survives_reopen_without_deleting_dictionary_word() {
    let path = TempDatabasePath::new("completion-hidden-word");
    {
        let database = Database::open(path.as_path()).unwrap();
        database.hide_completion_word("HELLO").unwrap();
    }

    let reopened = Database::open(path.as_path()).unwrap();
    let hidden = reopened.load_hidden_completion_words().unwrap();
    assert!(hidden.contains("hello"));
    let packs = reopened.load_enabled_language_packs().unwrap();
    assert!(
        packs
            .iter()
            .any(|pack| pack.id().as_str() == "en" && pack.contains_normalized("hello"))
    );
}

#[test]
fn certification_schema_v1_migrates_through_current_schema() {
    let path = TempDatabasePath::new("schema-v1-to-v2");
    {
        let database = Database::open(path.as_path()).unwrap();
        assert_eq!(database.schema_version().unwrap(), 6);
    }

    let raw = Connection::open(path.as_path()).unwrap();
    raw.execute("DROP TABLE completion_hidden_words", [])
        .unwrap();
    raw.pragma_update(None, "user_version", 1).unwrap();
    drop(raw);

    let migrated = Database::open(path.as_path()).unwrap();
    assert_eq!(migrated.schema_version().unwrap(), 6);
    migrated.hide_completion_word("hello").unwrap();
    assert!(
        migrated
            .load_hidden_completion_words()
            .unwrap()
            .contains("hello")
    );
}

#[test]
fn certification_schema_v2_removes_legacy_grave_pollution_on_upgrade() {
    let path = TempDatabasePath::new("schema-v2-grave-cleanup");
    {
        let database = Database::open(path.as_path()).unwrap();
        assert_eq!(database.schema_version().unwrap(), 6);
    }

    let raw = Connection::open(path.as_path()).unwrap();
    raw.execute(
        "INSERT INTO user_words (term, normalized_term, use_count, last_used_at_ms) VALUES ('`него', '`него', 2, 100)",
        [],
    )
    .unwrap();
    raw.execute(
        "INSERT INTO text_history (text, last_used_at_ms, use_count) VALUES ('hello `него', 100, 2)",
        [],
    )
    .unwrap();
    raw.pragma_update(None, "user_version", 2).unwrap();
    drop(raw);

    let migrated = Database::open(path.as_path()).unwrap();
    assert_eq!(migrated.schema_version().unwrap(), 6);
    assert!(
        migrated
            .load_user_lexicon()
            .unwrap()
            .exact("`него")
            .is_none()
    );
    assert!(
        migrated
            .load_text_history()
            .unwrap()
            .entries()
            .iter()
            .all(|entry| !entry.text().contains('`'))
    );
}

#[test]
fn certification_schema_v4_drops_obsolete_dictionary_table() {
    let path = TempDatabasePath::new("schema-v4-dictionary-drop");
    {
        let database = Database::open(path.as_path()).unwrap();
        assert_eq!(database.schema_version().unwrap(), 6);
    }

    let raw = Connection::open(path.as_path()).unwrap();
    raw.execute(
        "CREATE TABLE dictionary_words (id INTEGER PRIMARY KEY, language_id INTEGER NOT NULL, term TEXT NOT NULL, normalized_term TEXT NOT NULL, frequency INTEGER NOT NULL)",
        [],
    )
    .unwrap();
    raw.execute(
        "INSERT INTO dictionary_words (language_id, term, normalized_term, frequency) VALUES (1, 'legacyru', 'legacyru', 10)",
        [],
    )
    .unwrap();
    raw.pragma_update(None, "user_version", 4).unwrap();
    drop(raw);

    let migrated = Database::open(path.as_path()).unwrap();
    assert_eq!(migrated.schema_version().unwrap(), 6);
    drop(migrated);
    let raw = Connection::open(path.as_path()).unwrap();
    let exists: i64 = raw
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'dictionary_words'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(exists, 0);
}

#[test]
fn certification_correction_undo_persists_original_as_user_word_across_reopen() {
    let path = TempDatabasePath::new("correction-undo");
    let event = {
        let database = Database::open(path.as_path()).unwrap();
        database
            .record_correction_event("QuantileEntryStrategy1", "QuantileEntryStrategy", 100)
            .unwrap()
    };

    {
        let mut database = Database::open(path.as_path()).unwrap();
        let plan = database.prepare_correction_undo(event).unwrap();
        assert_eq!(plan.original_text(), "QuantileEntryStrategy1");
        assert_eq!(plan.replacement_text(), "QuantileEntryStrategy");
        database.commit_correction_undo(event, 200).unwrap();
    }

    let reopened = Database::open(path.as_path()).unwrap();
    let lexicon = reopened.load_user_lexicon().unwrap();
    let learned = lexicon.exact("quantileentrystrategy1").unwrap();
    assert_eq!(learned.term(), "QuantileEntryStrategy1");
    assert!(matches!(
        reopened.prepare_correction_undo(event),
        Err(DatabaseError::CorrectionEventAlreadyUndone(_))
    ));
}

#[test]
fn certification_settings_are_canonical_and_survive_reopen() {
    let path = TempDatabasePath::new("settings");
    {
        let database = Database::open(path.as_path()).unwrap();
        let limit = ClipboardHistoryLimit::try_new(777).unwrap();
        database
            .save_settings(AppSettings::new(limit, UndoHotkey::Pause))
            .unwrap();
    }

    let reopened = Database::open(path.as_path()).unwrap();
    let settings = reopened.settings().unwrap();
    assert_eq!(settings.clipboard_history_limit().get(), 777);
    assert_eq!(settings.undo_hotkey(), UndoHotkey::Pause);
}

#[test]
fn certification_clipboard_child_rows_are_owned_by_parent_entry() {
    let path = TempDatabasePath::new("clipboard-foreign-keys");
    let database = Database::open(path.as_path()).unwrap();
    drop(database);

    let raw = Connection::open(path.as_path()).unwrap();
    raw.pragma_update(None, "foreign_keys", "ON").unwrap();
    raw.execute(
        "INSERT INTO clipboard_entries (id, kind, last_seen_at_ms, content_hash) VALUES (5, 'files', 1, X'01')",
        [],
    )
    .unwrap();
    raw.execute(
        "INSERT INTO clipboard_files (entry_id, position, path) VALUES (5, 0, 'C:\\a.txt'), (5, 1, 'C:\\b.txt')",
        [],
    )
    .unwrap();

    raw.execute("DELETE FROM clipboard_entries WHERE id = 5", [])
        .unwrap();

    let file_count: i64 = raw
        .query_row("SELECT count(*) FROM clipboard_files", [], |row| row.get(0))
        .unwrap();
    assert_eq!(file_count, 0);
}

#[test]
fn certification_newer_database_schema_fails_closed() {
    let path = TempDatabasePath::new("future-schema");
    let raw = Connection::open(path.as_path()).unwrap();
    raw.pragma_update(None, "user_version", 999).unwrap();
    drop(raw);

    let error = match Database::open(path.as_path()) {
        Ok(_) => panic!("newer schema must not be opened by an older binary"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        DatabaseError::SchemaTooNew {
            found: 999,
            supported: 6
        }
    ));
}
