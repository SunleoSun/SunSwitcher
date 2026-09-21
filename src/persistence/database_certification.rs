use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use crate::lexicon::UserTermProtection;

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
    assert_eq!(database.schema_version().unwrap(), 4);
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
            "correction_events",
            "dictionary_words",
            "languages",
            "text_history",
            "user_terms",
        ]
    );
    assert!(!tables.iter().any(|table| table == "schema_migrations"));
}

#[test]
fn certification_user_terms_are_language_neutral_in_canonical_schema() {
    let path = TempDatabasePath::new("language-neutral-user-terms");
    let database = Database::open(path.as_path()).unwrap();
    drop(database);

    let raw = Connection::open(path.as_path()).unwrap();
    let mut statement = raw.prepare("PRAGMA table_info(user_terms)").unwrap();
    let columns: Vec<String> = statement
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!columns.iter().any(|column| column == "language_id"));
}

#[test]
fn certification_schema_v2_migrates_to_default_pause_undo_hotkey() {
    let path = TempDatabasePath::new("v2-undo-hotkey");
    let raw = Connection::open(path.as_path()).unwrap();
    raw.execute_batch(
        "CREATE TABLE app_settings (singleton INTEGER PRIMARY KEY CHECK (singleton = 1), clipboard_history_limit INTEGER NOT NULL CHECK (clipboard_history_limit > 0)) STRICT;\nINSERT INTO app_settings (singleton, clipboard_history_limit) VALUES (1, 321);\nCREATE TABLE user_terms (id INTEGER PRIMARY KEY, term TEXT NOT NULL CHECK (length(term) > 0), normalized_term TEXT NOT NULL UNIQUE CHECK (length(normalized_term) > 0), language_id INTEGER, protected INTEGER NOT NULL DEFAULT 0 CHECK (protected IN (0, 1)), use_count INTEGER NOT NULL DEFAULT 1 CHECK (use_count > 0), last_used_at_ms INTEGER NOT NULL) STRICT;\nPRAGMA user_version = 2;",
    )
    .unwrap();
    drop(raw);

    let database = Database::open(path.as_path()).unwrap();
    assert_eq!(database.schema_version().unwrap(), 4);
    let settings = database.settings().unwrap();
    assert_eq!(settings.clipboard_history_limit().get(), 321);
    assert_eq!(settings.undo_hotkey(), UndoHotkey::Pause);
}

#[test]
fn certification_schema_v3_migrates_pause_and_language_neutral_user_terms_without_data_loss() {
    let path = TempDatabasePath::new("v3-pause-user-terms");
    let raw = Connection::open(path.as_path()).unwrap();
    raw.execute_batch(
        "CREATE TABLE app_settings (singleton INTEGER PRIMARY KEY CHECK (singleton = 1), clipboard_history_limit INTEGER NOT NULL CHECK (clipboard_history_limit > 0), undo_hotkey TEXT NOT NULL DEFAULT 'print_screen') STRICT;\nINSERT INTO app_settings (singleton, clipboard_history_limit, undo_hotkey) VALUES (1, 444, 'print_screen');\nCREATE TABLE user_terms (id INTEGER PRIMARY KEY, term TEXT NOT NULL CHECK (length(term) > 0), normalized_term TEXT NOT NULL UNIQUE CHECK (length(normalized_term) > 0), language_id INTEGER, protected INTEGER NOT NULL DEFAULT 0 CHECK (protected IN (0, 1)), use_count INTEGER NOT NULL DEFAULT 1 CHECK (use_count > 0), last_used_at_ms INTEGER NOT NULL) STRICT;\nINSERT INTO user_terms (id, term, normalized_term, language_id, protected, use_count, last_used_at_ms) VALUES (7, 'QuantileEntryStrategy', 'quantileentrystrategy', 42, 1, 3, 900);\nPRAGMA user_version = 3;",
    )
    .unwrap();
    drop(raw);

    let database = Database::open(path.as_path()).unwrap();
    assert_eq!(database.schema_version().unwrap(), 4);
    let settings = database.settings().unwrap();
    assert_eq!(settings.clipboard_history_limit().get(), 444);
    assert_eq!(settings.undo_hotkey(), UndoHotkey::Pause);
    let lexicon = database.load_user_lexicon().unwrap();
    let term = lexicon.exact("quantileentrystrategy").unwrap();
    assert_eq!(term.term(), "QuantileEntryStrategy");
    assert_eq!(term.use_count(), 3);
    assert_eq!(term.last_used_at_ms(), 900);
    assert_eq!(term.protection(), UserTermProtection::Protected);
    drop(database);

    let raw = Connection::open(path.as_path()).unwrap();
    let mut statement = raw.prepare("PRAGMA table_info(user_terms)").unwrap();
    let columns: Vec<String> = statement
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!columns.iter().any(|column| column == "language_id"));
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
fn certification_seed_dictionaries_build_runtime_language_packs() {
    let database = Database::open_in_memory().unwrap();
    let packs = database.load_enabled_language_packs().unwrap();
    assert_eq!(packs.len(), 2);

    let russian = packs
        .iter()
        .find(|pack| pack.id().as_str() == "ru")
        .expect("Russian seed language must be enabled");
    assert!(russian.contains_normalized("для"));
    assert!(russian.contains_normalized("жизнь"));
    assert_eq!(russian.transforms().len(), 1);

    let english = packs
        .iter()
        .find(|pack| pack.id().as_str() == "en")
        .expect("English seed language must be enabled");
    assert!(english.contains_normalized("hello"));
    assert!(english.contains_normalized("world"));
    assert_eq!(english.transforms().len(), 1);
}

#[test]
fn certification_user_terms_survive_reopen_and_rebuild_ranked_runtime_snapshot() {
    let path = TempDatabasePath::new("user-terms");
    {
        let database = Database::open(path.as_path()).unwrap();
        database
            .record_user_term("QuantileEntryStrategy", UserTermProtection::Normal, 100)
            .unwrap();
        database
            .record_user_term("QuantileEntryStrategy1", UserTermProtection::Protected, 200)
            .unwrap();
        database
            .record_user_term("quantileentrystrategy1", UserTermProtection::Normal, 150)
            .unwrap();
    }

    let reopened = Database::open(path.as_path()).unwrap();
    let lexicon = reopened.load_user_lexicon().unwrap();
    let protected = lexicon.exact("quantileentrystrategy1").unwrap();
    assert_eq!(protected.term(), "QuantileEntryStrategy1");
    assert_eq!(protected.protection(), UserTermProtection::Protected);
    assert_eq!(protected.use_count(), 2);
    assert_eq!(protected.last_used_at_ms(), 200);
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
fn certification_correction_undo_persists_protected_original_across_reopen() {
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
    let protected = lexicon.exact("quantileentrystrategy1").unwrap();
    assert_eq!(protected.term(), "QuantileEntryStrategy1");
    assert_eq!(protected.protection(), UserTermProtection::Protected);
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
fn certification_foreign_keys_own_dictionary_and_clipboard_child_lifecycles() {
    let path = TempDatabasePath::new("foreign-keys");
    let database = Database::open(path.as_path()).unwrap();
    drop(database);

    let raw = Connection::open(path.as_path()).unwrap();
    raw.pragma_update(None, "foreign_keys", "ON").unwrap();
    raw.execute(
        "INSERT INTO languages (code, display_name) VALUES ('zz-test', 'Test')",
        [],
    )
    .unwrap();
    let test_language_id = raw.last_insert_rowid();
    raw.execute(
        "INSERT INTO dictionary_words (language_id, term, normalized_term, frequency) VALUES (?1, 'cascade_probe', 'cascade_probe', 10)",
        [test_language_id],
    )
    .unwrap();
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

    raw.execute("DELETE FROM languages WHERE id = ?1", [test_language_id])
        .unwrap();
    raw.execute("DELETE FROM clipboard_entries WHERE id = 5", [])
        .unwrap();

    let dictionary_count: i64 = raw
        .query_row(
            "SELECT count(*) FROM dictionary_words WHERE term = 'cascade_probe'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let file_count: i64 = raw
        .query_row("SELECT count(*) FROM clipboard_files", [], |row| row.get(0))
        .unwrap();
    assert_eq!(dictionary_count, 0);
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
            supported: 4
        }
    ));
}
