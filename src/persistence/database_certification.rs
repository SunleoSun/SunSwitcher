use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use super::*;
use sun_observations::certifications::*;

#[test]
fn certification_sequence_reobservation_renews_retention_from_latest_use() {
    let mut c = certification(
        "sequence reobservation renews inactivity retention without resetting evidence",
    );
    let mut database = Database::open_in_memory().unwrap();
    database
        .record_text_history("hello world", RETENTION_CUTOFFS_MS[0] + 1)
        .unwrap();
    database
        .record_text_history("hello world", RETENTION_NOW_MS)
        .unwrap();
    database
        .record_text_history("unrenewed evidence", RETENTION_CUTOFFS_MS[0] + 1)
        .unwrap();
    database
        .record_text_history("unrenewed evidence", RETENTION_CUTOFFS_MS[0] + 1)
        .unwrap();
    let report = database
        .maintain(RETENTION_NOW_MS + 45 * 86_400_000)
        .unwrap();
    let history = database.load_text_history().unwrap();
    c.check_fact_expect(
        "only unrenewed evidence expires",
        report.text_history_deleted,
        1,
    );
    c.check_fact_expect(
        "usage and latest timestamp preserved",
        history
            .entries()
            .iter()
            .map(|r| (r.use_count(), r.last_used_at_ms()))
            .collect::<Vec<_>>(),
        vec![(2, RETENTION_NOW_MS)],
    );
    c.assert_certify();
}

#[test]
fn certification_exclusive_vacuum_rejects_concurrent_reader_without_losing_data() {
    let mut c = certification("offline compaction fails closed under competing access");
    let path = TempDatabasePath::new("vacuum-concurrent-reader");
    let database = Database::open(path.as_path()).unwrap();
    database.record_user_word("preservedword", 10).unwrap();
    let reader = Connection::open(path.as_path()).unwrap();
    reader
        .execute_batch("BEGIN; SELECT * FROM user_words;")
        .unwrap();
    c.check_fact_expect(
        "competing reader rejects exclusive compaction",
        matches!(database.vacuum_exclusive(), Err(DatabaseError::Sqlite(_))),
        true,
    );
    drop(reader);
    drop(database);
    let reopened = Database::open_existing(path.as_path()).unwrap();
    c.check_fact_expect(
        "rejected compaction preserves user data",
        reopened
            .load_user_lexicon()
            .unwrap()
            .exact("preservedword")
            .map(|r| (r.use_count(), r.last_used_at_ms())),
        Some((1, 10)),
    );
    reopened.vacuum_exclusive().unwrap();
    c.assert_certify();
}

#[test]
fn certification_offline_open_never_creates_a_missing_database() {
    let mut c = certification("offline maintenance cannot create an empty replacement database");
    let path = TempDatabasePath::new("missing-offline-database");
    c.check_fact_expect(
        "missing file rejected",
        Database::open_existing(path.as_path()).is_err(),
        true,
    );
    c.check_fact_expect("no database created", path.as_path().exists(), false);
    c.assert_certify();
}

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
    let mut c =
        certification("certification_fresh_database_has_only_the_required_application_tables");
    let path = TempDatabasePath::new("schema");
    let database = Database::open(path.as_path()).unwrap();
    {
        let observed = &(database.schema_version().unwrap());
        let expected = &(10);
        c.check(
            stringify!(database.schema_version().unwrap()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
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

    {
        let observed = &(tables);
        let expected = &([
            "app_settings",
            "application_replacement_suppressions",
            "clipboard_entries",
            "clipboard_files",
            "clipboard_images",
            "clipboard_text",
            "completion_hidden_words",
            "correction_events",
            "ignored_words",
            "languages",
            "text_history",
            "user_words",
        ]);
        c.check(
            stringify!(tables),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.check(
        stringify!(!tables.iter().any(|table| table == "schema_migrations")),
        fact!(!tables.iter().any(|table| table == "schema_migrations")).expect(true),
    );
    c.assert_certify();
}

#[test]
fn certification_user_words_are_language_neutral_in_canonical_schema() {
    let mut c = certification("certification_user_words_are_language_neutral_in_canonical_schema");
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
    c.check(
        stringify!(!columns.iter().any(|column| column == "language_id")),
        fact!(!columns.iter().any(|column| column == "language_id")).expect(true),
    );
    c.check(
        stringify!(!columns.iter().any(|column| column == "protected")),
        fact!(!columns.iter().any(|column| column == "protected")).expect(true),
    );
    c.assert_certify();
}

#[test]
fn certification_unknown_stored_undo_hotkey_fails_closed() {
    let mut c = certification("certification_unknown_stored_undo_hotkey_fails_closed");
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
    c.check(
        stringify!(matches!(
            reopened.settings(),
            Err(DatabaseError::InvalidStoredUndoHotkey(value)) if value == "future_key"
        )),
        fact!(matches!(
            reopened.settings(),
            Err(DatabaseError::InvalidStoredUndoHotkey(value)) if value == "future_key"
        ))
        .expect(true),
    );
    c.assert_certify();
}

#[test]
fn certification_dictionary_surface_rows_build_runtime_language_packs() {
    let mut c = certification("certification_dictionary_surface_rows_build_runtime_language_packs");
    let database = Database::open_in_memory().unwrap();
    let packs = database.load_enabled_language_packs().unwrap();
    {
        let observed = &(packs.len());
        let expected = &(2);
        c.check(
            stringify!(packs.len()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };

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
        c.check(
            stringify!(russian.contains_normalized(word)),
            fact!(russian.contains_normalized(word)).expect(true),
        );
    }
    {
        let observed = &(russian.transforms().len());
        let expected = &(1);
        c.check(
            stringify!(russian.transforms().len()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };

    let english = packs
        .iter()
        .find(|pack| pack.id().as_str() == "en")
        .expect("English dictionary language must be enabled");
    for word in [
        "hello", "world", "works", "worked", "working", "tries", "tried", "children",
    ] {
        c.check(
            stringify!(english.contains_normalized(word)),
            fact!(english.contains_normalized(word)).expect(true),
        );
    }
    {
        let observed = &(english.transforms().len());
        let expected = &(1);
        c.check(
            stringify!(english.transforms().len()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn certification_builtin_vocabulary_is_not_duplicated_in_sqlite() {
    let mut c = certification("certification_builtin_vocabulary_is_not_duplicated_in_sqlite");
    let path = TempDatabasePath::new("builtin-dictionary-authority");
    {
        let database = Database::open(path.as_path()).unwrap();
        {
            let observed = &(database.schema_version().unwrap());
            let expected = &(10);
            c.check(
                stringify!(database.schema_version().unwrap()),
                fact("persisted evidence", observed == expected)
                    .details(format!("observed={observed:?}; expected={expected:?}"))
                    .expect(true),
            );
        };
    }

    let raw = Connection::open(path.as_path()).unwrap();
    let exists: i64 = raw
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'dictionary_words'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    {
        let observed = &(exists);
        let expected = &(0);
        c.check(
            stringify!(exists),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn certification_unknown_enabled_language_fails_closed() {
    let mut c = certification("certification_unknown_enabled_language_fails_closed");
    let path = TempDatabasePath::new("unknown-language");
    {
        let database = Database::open(path.as_path()).unwrap();
        {
            let observed = &(database.schema_version().unwrap());
            let expected = &(10);
            c.check(
                stringify!(database.schema_version().unwrap()),
                fact("persisted evidence", observed == expected)
                    .details(format!("observed={observed:?}; expected={expected:?}"))
                    .expect(true),
            );
        };
    }

    let raw = Connection::open(path.as_path()).unwrap();
    raw.execute(
        "INSERT INTO languages (code, display_name, enabled) VALUES ('xx', 'Custom', 1)",
        [],
    )
    .unwrap();
    drop(raw);

    let database = Database::open(path.as_path()).unwrap();
    c.check(
        stringify!(matches!(
            database.load_enabled_language_packs(),
            Err(DatabaseError::InvalidStoredLanguageCode(code)) if code == "xx"
        )),
        fact!(matches!(
            database.load_enabled_language_packs(),
            Err(DatabaseError::InvalidStoredLanguageCode(code)) if code == "xx"
        ))
        .expect(true),
    );
    c.assert_certify();
}

#[test]
fn certification_user_words_survive_reopen_and_rebuild_ranked_runtime_snapshot() {
    let mut c = certification(
        "certification_user_words_survive_reopen_and_rebuild_ranked_runtime_snapshot",
    );
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
    {
        let observed = &(learned.term());
        let expected = &("QuantileEntryStrategy1");
        c.check(
            stringify!(learned.term()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(learned.use_count());
        let expected = &(2);
        c.check(
            stringify!(learned.use_count()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(learned.last_used_at_ms());
        let expected = &(200);
        c.check(
            stringify!(learned.last_used_at_ms()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(lexicon
            .prefix_matches("QuantileEntry", 2)
            .iter()
            .map(|entry| entry.term())
            .collect::<Vec<_>>());
        let expected = &(["QuantileEntryStrategy1", "QuantileEntryStrategy"]);
        c.check(
            stringify!(
                lexicon
                    .prefix_matches("QuantileEntry", 2)
                    .iter()
                    .map(|entry| entry.term())
                    .collect::<Vec<_>>()
            ),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn certification_text_history_accumulates_repetition_and_survives_reopen() {
    let mut c =
        certification("certification_text_history_accumulates_repetition_and_survives_reopen");
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
    {
        let observed = &(entry.use_count());
        let expected = &(2);
        c.check(
            stringify!(entry.use_count()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(entry.last_used_at_ms());
        let expected = &(200);
        c.check(
            stringify!(entry.last_used_at_ms()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn certification_hidden_completion_word_survives_reopen_without_deleting_dictionary_word() {
    let mut c = certification(
        "certification_hidden_completion_word_survives_reopen_without_deleting_dictionary_word",
    );
    let path = TempDatabasePath::new("completion-hidden-word");
    {
        let database = Database::open(path.as_path()).unwrap();
        database.hide_completion_word("HELLO").unwrap();
    }

    let reopened = Database::open(path.as_path()).unwrap();
    let hidden = reopened.load_hidden_completion_words().unwrap();
    c.check(
        stringify!(hidden.contains("hello")),
        fact!(hidden.contains("hello")).expect(true),
    );
    let packs = reopened.load_enabled_language_packs().unwrap();
    c.check(
        stringify!(
            packs
                .iter()
                .any(|pack| pack.id().as_str() == "en" && pack.contains_normalized("hello"))
        ),
        fact!(
            packs
                .iter()
                .any(|pack| pack.id().as_str() == "en" && pack.contains_normalized("hello"))
        )
        .expect(true),
    );
    c.assert_certify();
}

#[test]
fn certification_schema_v1_migrates_through_current_schema() {
    let mut c = certification("certification_schema_v1_migrates_through_current_schema");
    let path = TempDatabasePath::new("schema-v1-to-v2");
    {
        let database = Database::open(path.as_path()).unwrap();
        {
            let observed = &(database.schema_version().unwrap());
            let expected = &(10);
            c.check(
                stringify!(database.schema_version().unwrap()),
                fact("persisted evidence", observed == expected)
                    .details(format!("observed={observed:?}; expected={expected:?}"))
                    .expect(true),
            );
        };
    }

    let raw = Connection::open(path.as_path()).unwrap();
    raw.execute("DROP TABLE completion_hidden_words", [])
        .unwrap();
    raw.pragma_update(None, "user_version", 1).unwrap();
    drop(raw);

    let migrated = Database::open(path.as_path()).unwrap();
    {
        let observed = &(migrated.schema_version().unwrap());
        let expected = &(10);
        c.check(
            stringify!(migrated.schema_version().unwrap()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    migrated.hide_completion_word("hello").unwrap();
    c.check(
        stringify!(
            migrated
                .load_hidden_completion_words()
                .unwrap()
                .contains("hello")
        ),
        fact!(
            migrated
                .load_hidden_completion_words()
                .unwrap()
                .contains("hello")
        )
        .expect(true),
    );
    c.assert_certify();
}

#[test]
fn certification_schema_v2_removes_legacy_grave_pollution_on_upgrade() {
    let mut c = certification("certification_schema_v2_removes_legacy_grave_pollution_on_upgrade");
    let path = TempDatabasePath::new("schema-v2-grave-cleanup");
    {
        let database = Database::open(path.as_path()).unwrap();
        {
            let observed = &(database.schema_version().unwrap());
            let expected = &(10);
            c.check(
                stringify!(database.schema_version().unwrap()),
                fact("persisted evidence", observed == expected)
                    .details(format!("observed={observed:?}; expected={expected:?}"))
                    .expect(true),
            );
        };
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
    {
        let observed = &(migrated.schema_version().unwrap());
        let expected = &(10);
        c.check(
            stringify!(migrated.schema_version().unwrap()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.check(
        stringify!(
            migrated
                .load_user_lexicon()
                .unwrap()
                .exact("`него")
                .is_none()
        ),
        fact!(
            migrated
                .load_user_lexicon()
                .unwrap()
                .exact("`него")
                .is_none()
        )
        .expect(true),
    );
    c.check(
        stringify!(
            migrated
                .load_text_history()
                .unwrap()
                .entries()
                .iter()
                .all(|entry| !entry.text().contains('`'))
        ),
        fact!(
            migrated
                .load_text_history()
                .unwrap()
                .entries()
                .iter()
                .all(|entry| !entry.text().contains('`'))
        )
        .expect(true),
    );
    c.assert_certify();
}

#[test]
fn certification_schema_v4_drops_obsolete_dictionary_table() {
    let mut c = certification("certification_schema_v4_drops_obsolete_dictionary_table");
    let path = TempDatabasePath::new("schema-v4-dictionary-drop");
    {
        let database = Database::open(path.as_path()).unwrap();
        {
            let observed = &(database.schema_version().unwrap());
            let expected = &(10);
            c.check(
                stringify!(database.schema_version().unwrap()),
                fact("persisted evidence", observed == expected)
                    .details(format!("observed={observed:?}; expected={expected:?}"))
                    .expect(true),
            );
        };
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
    {
        let observed = &(migrated.schema_version().unwrap());
        let expected = &(10);
        c.check(
            stringify!(migrated.schema_version().unwrap()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    drop(migrated);
    let raw = Connection::open(path.as_path()).unwrap();
    let exists: i64 = raw
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'dictionary_words'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    {
        let observed = &(exists);
        let expected = &(0);
        c.check(
            stringify!(exists),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn certification_schema_v7_adds_persistent_ignored_words() {
    let mut c = certification("certification_schema_v7_adds_persistent_ignored_words");
    let path = TempDatabasePath::new("schema-v6-ignored-words");
    {
        let database = Database::open(path.as_path()).unwrap();
        {
            let observed = &(database.schema_version().unwrap());
            let expected = &(10);
            c.check(
                stringify!(database.schema_version().unwrap()),
                fact("persisted evidence", observed == expected)
                    .details(format!("observed={observed:?}; expected={expected:?}"))
                    .expect(true),
            );
        };
    }

    let raw = Connection::open(path.as_path()).unwrap();
    raw.execute("DROP TABLE ignored_words", []).unwrap();
    raw.pragma_update(None, "user_version", 6).unwrap();
    drop(raw);

    let mut migrated = Database::open(path.as_path()).unwrap();
    {
        let observed = &(migrated.schema_version().unwrap());
        let expected = &(10);
        c.check(
            stringify!(migrated.schema_version().unwrap()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.check(
        stringify!(migrated.load_ignored_words().unwrap().is_empty()),
        fact!(migrated.load_ignored_words().unwrap().is_empty()).expect(true),
    );
    migrated.ignore_word("jr").unwrap();
    drop(migrated);

    let reopened = Database::open(path.as_path()).unwrap();
    c.check(
        stringify!(
            reopened
                .load_ignored_words()
                .unwrap()
                .contains_normalized("jr")
        ),
        fact!(
            reopened
                .load_ignored_words()
                .unwrap()
                .contains_normalized("jr")
        )
        .expect(true),
    );
    c.assert_certify();
}

#[test]
fn certification_correction_undo_persists_original_as_user_word_across_reopen() {
    let mut c =
        certification("certification_correction_undo_persists_original_as_user_word_across_reopen");
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
        {
            let observed = &(plan.original_text());
            let expected = &("QuantileEntryStrategy1");
            c.check(
                stringify!(plan.original_text()),
                fact("persisted evidence", observed == expected)
                    .details(format!("observed={observed:?}; expected={expected:?}"))
                    .expect(true),
            );
        };
        {
            let observed = &(plan.replacement_text());
            let expected = &("QuantileEntryStrategy");
            c.check(
                stringify!(plan.replacement_text()),
                fact("persisted evidence", observed == expected)
                    .details(format!("observed={observed:?}; expected={expected:?}"))
                    .expect(true),
            );
        };
        database.commit_correction_undo(event, 200).unwrap();
    }

    let reopened = Database::open(path.as_path()).unwrap();
    let lexicon = reopened.load_user_lexicon().unwrap();
    let learned = lexicon.exact("quantileentrystrategy1").unwrap();
    {
        let observed = &(learned.term());
        let expected = &("QuantileEntryStrategy1");
        c.check(
            stringify!(learned.term()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.check(
        stringify!(matches!(
            reopened.prepare_correction_undo(event),
            Err(DatabaseError::CorrectionEventAlreadyUndone(_))
        )),
        fact!(matches!(
            reopened.prepare_correction_undo(event),
            Err(DatabaseError::CorrectionEventAlreadyUndone(_))
        ))
        .expect(true),
    );
    c.assert_certify();
}

#[test]
fn certification_settings_are_canonical_and_survive_reopen() {
    let mut c = certification("certification_settings_are_canonical_and_survive_reopen");
    let path = TempDatabasePath::new("settings");
    let expected = AppSettings::new(
        ClipboardHistoryLimit::try_new(777).unwrap(),
        UndoHotkey::Pause,
    )
    .with_hotkey(
        HotkeyAction::IgnoreWord,
        HotkeyBinding::try_new(b'I' as u16, true, true, false, false).unwrap(),
    )
    .with_enable_autocomplete(false)
    .with_enable_autocorrections(false)
    .with_enable_auto_keyboard_switches(true)
    .with_notification_timeout_seconds(NotificationTimeoutSeconds::try_new(7).unwrap());
    {
        let database = Database::open(path.as_path()).unwrap();
        database.save_settings(expected).unwrap();
    }

    let reopened = Database::open(path.as_path()).unwrap();
    {
        let observed = &(reopened.settings().unwrap());
        let expected = &(expected);
        c.check(
            stringify!(reopened.settings().unwrap()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn certification_application_overrides_round_trip_deduplicate_remove_and_reopen() {
    let mut c = certification(
        "certification_application_overrides_round_trip_deduplicate_remove_and_reopen",
    );
    use crate::application_policy::ApplicationIdentity;
    let path = TempDatabasePath::new("application-overrides");
    let application = ApplicationIdentity::try_new(r"C:\Apps\Example.exe").unwrap();
    let other = ApplicationIdentity::try_new(r"D:\Apps\Example.exe").unwrap();
    {
        let database = Database::open(path.as_path()).unwrap();
        c.check_fact_expect(
            "fresh overrides are empty",
            database
                .load_application_overrides()
                .unwrap()
                .applications()
                .count(),
            0,
        );
        database
            .set_application_suppressed(&application, true)
            .unwrap();
        database
            .set_application_suppressed(
                &ApplicationIdentity::try_new("c:/apps/EXAMPLE.EXE").unwrap(),
                true,
            )
            .unwrap();
        let overrides = database.load_application_overrides().unwrap();
        c.check_fact_expect(
            "normalized duplicates persist one row",
            overrides.applications().count(),
            1,
        );
        c.check_fact_expect(
            "configured path suppresses",
            overrides.suppresses(&application),
            true,
        );
        c.check_fact_expect(
            "same basename on another drive inherits",
            !overrides.suppresses(&other),
            true,
        );
        c.check_fact_expect(
            "application writes preserve all globals",
            database.settings().unwrap(),
            AppSettings::default(),
        );
    }
    {
        let database = Database::open(path.as_path()).unwrap();
        c.check_fact_expect(
            "suppression survives reopen",
            database
                .load_application_overrides()
                .unwrap()
                .suppresses(&application),
            true,
        );
        database
            .set_application_suppressed(&application, false)
            .unwrap();
        database
            .set_application_suppressed(&application, false)
            .unwrap();
    }
    c.check_fact_expect(
        "idempotent removal survives reopen",
        Database::open(path.as_path())
            .unwrap()
            .load_application_overrides()
            .unwrap()
            .applications()
            .count(),
        0,
    );
    c.assert_certify();
}

#[test]
fn certification_v8_upgrades_application_relation_without_changing_global_settings() {
    let mut c = certification(
        "certification_v8_upgrades_application_relation_without_changing_global_settings",
    );
    let path = TempDatabasePath::new("v8-applications");
    let expected = AppSettings::default().with_enable_autocorrections(false);
    {
        let database = Database::open(path.as_path()).unwrap();
        database.save_settings(expected).unwrap();
    }
    {
        let raw = Connection::open(path.as_path()).unwrap();
        raw.execute_batch(
            "DROP TABLE application_replacement_suppressions; PRAGMA user_version = 8;",
        )
        .unwrap();
    }
    for _ in 0..2 {
        let database = Database::open(path.as_path()).unwrap();
        c.check_fact_expect(
            "v8 migrates to current schema",
            database.schema_version().unwrap(),
            10,
        );
        c.check_fact_expect(
            "migration preserves global settings",
            database.settings().unwrap(),
            expected,
        );
        c.check_fact_expect(
            "v8 starts without overrides",
            database
                .load_application_overrides()
                .unwrap()
                .applications()
                .count(),
            0,
        );
    }
    c.assert_certify();
}

#[test]
fn certification_invalid_or_noncanonical_application_rows_fail_closed() {
    let mut c = certification("certification_invalid_or_noncanonical_application_rows_fail_closed");
    for invalid in [
        "example.exe",
        r"C:\Apps\..\example.exe",
        r"C:\Apps\Example.exe",
        "c:/apps/example.exe",
        r"c:\apps\example.exe:stream",
    ] {
        let path = TempDatabasePath::new("invalid-application");
        drop(Database::open(path.as_path()).unwrap());
        let raw = Connection::open(path.as_path()).unwrap();
        raw.execute(
            "INSERT INTO application_replacement_suppressions VALUES (?1)",
            [invalid],
        )
        .unwrap();
        drop(raw);
        let database = Database::open(path.as_path()).unwrap();
        c.check_fact_expect(
            "malformed row rejected by identity owner",
            matches!(
                database.load_application_overrides(),
                Err(DatabaseError::InvalidStoredApplicationIdentity(_))
            ),
            true,
        );
        c.check_fact_expect(
            "malformed row prevents runtime projection",
            crate::settings::SettingsStore::from_database(database).is_err(),
            true,
        );
    }
    c.assert_certify();
}

#[test]
fn certification_clipboard_child_rows_are_owned_by_parent_entry() {
    let mut c = certification("certification_clipboard_child_rows_are_owned_by_parent_entry");
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
    {
        let observed = &(file_count);
        let expected = &(0);
        c.check(
            stringify!(file_count),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn certification_newer_database_schema_fails_closed() {
    let mut c = certification("certification_newer_database_schema_fails_closed");
    let path = TempDatabasePath::new("future-schema");
    let raw = Connection::open(path.as_path()).unwrap();
    raw.pragma_update(None, "user_version", 999).unwrap();
    drop(raw);

    let error = match Database::open(path.as_path()) {
        Ok(_) => panic!("newer schema must not be opened by an older binary"),
        Err(error) => error,
    };
    c.check(
        stringify!(matches!(
            error,
            DatabaseError::SchemaTooNew {
                found: 999,
                supported: 10
            }
        )),
        fact!(matches!(
            error,
            DatabaseError::SchemaTooNew {
                found: 999,
                supported: 10
            }
        ))
        .expect(true),
    );
    c.assert_certify();
}

#[test]
fn file_database_uses_wal_normal_and_in_memory_keeps_memory_journal() {
    let mut c = certification("file_database_uses_wal_normal_and_in_memory_keeps_memory_journal");
    use std::time::{SystemTime, UNIX_EPOCH};

    let memory = Database::open_in_memory().unwrap();
    let (memory_journal, _) = storage_pragmas(&memory).unwrap();
    {
        let observed = &(memory_journal);
        let expected = &("memory");
        c.check(
            stringify!(memory_journal),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sunswitcher-storage-{}-{nonce}.db",
        std::process::id()
    ));
    let database = Database::open(&path).unwrap();
    let (journal, synchronous) = storage_pragmas(&database).unwrap();
    {
        let observed = &(journal);
        let expected = &("wal");
        c.check(
            stringify!(journal),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(synchronous);
        let expected = &(1);
        c.check(
            stringify!(synchronous),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    drop(database);

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    c.assert_certify();
}

#[test]
fn default_settings_use_the_explicit_clipboard_history_limit() {
    let mut c = certification("default_settings_use_the_explicit_clipboard_history_limit");
    let database = Database::open_in_memory().expect("fresh in-memory database should open");

    {
        let observed = &(database.schema_version().unwrap());
        let expected = &(10);
        c.check(
            stringify!(database.schema_version().unwrap()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(database.settings().unwrap().clipboard_history_limit());
        let expected = &(ClipboardHistoryLimit::DEFAULT);
        c.check(
            stringify!(database.settings().unwrap().clipboard_history_limit()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(ClipboardHistoryLimit::DEFAULT.get());
        let expected = &(1000);
        c.check(
            stringify!(ClipboardHistoryLimit::DEFAULT.get()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(database.settings().unwrap().undo_hotkey());
        let expected = &(UndoHotkey::Pause);
        c.check(
            stringify!(database.settings().unwrap().undo_hotkey()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(UndoHotkey::DEFAULT);
        let expected = &(UndoHotkey::Pause);
        c.check(
            stringify!(UndoHotkey::DEFAULT),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn clipboard_history_limit_is_typed_and_round_trips_through_sqlite() {
    let mut c = certification("clipboard_history_limit_is_typed_and_round_trips_through_sqlite");
    {
        let observed = &(ClipboardHistoryLimit::try_new(0));
        let expected = &(Err(SettingsError::ClipboardHistoryLimitMustBePositive));
        c.check(
            stringify!(ClipboardHistoryLimit::try_new(0)),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };

    let database = Database::open_in_memory().unwrap();
    let limit = ClipboardHistoryLimit::try_new(250).unwrap();
    database
        .save_settings(AppSettings::new(limit, UndoHotkey::Pause))
        .unwrap();

    {
        let observed = &(database.settings().unwrap().clipboard_history_limit());
        let expected = &(limit);
        c.check(
            stringify!(database.settings().unwrap().clipboard_history_limit()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn hotkey_conflicts_include_legacy_clipboard_key_aliases() {
    let mut c = certification("hotkey_conflicts_include_legacy_clipboard_key_aliases");
    let settings = AppSettings::default();
    let current_oem_minus = HotkeyBinding::try_new(0xBD, true, true, false, false).unwrap();
    let pinned_oem_plus = HotkeyBinding::try_new(0xBB, true, true, false, false).unwrap();

    {
        let observed =
            &(settings.hotkey_conflict(HotkeyAction::CompletionAccept, current_oem_minus));
        let expected = &(Some(HotkeyConflict::Configured(HotkeyAction::ClipboardCurrent)));
        c.check(
            stringify!(settings.hotkey_conflict(HotkeyAction::CompletionAccept, current_oem_minus)),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(settings.hotkey_conflict(HotkeyAction::CompletionAccept, pinned_oem_plus));
        let expected = &(Some(HotkeyConflict::Configured(HotkeyAction::ClipboardPinned)));
        c.check(
            stringify!(settings.hotkey_conflict(HotkeyAction::CompletionAccept, pinned_oem_plus)),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn fixed_completion_controls_are_reserved_from_configurable_hotkeys() {
    let mut c = certification("fixed_completion_controls_are_reserved_from_configurable_hotkeys");
    let settings = AppSettings::default();
    for key_code in [0x1B, 0x26, 0x28, 0x2E] {
        let binding = HotkeyBinding::try_new(key_code, false, false, false, false).unwrap();
        {
            let observed = &(settings.hotkey_conflict(HotkeyAction::CompletionAccept, binding));
            let expected = &(Some(HotkeyConflict::ReservedCompletionControl));
            c.check(
                stringify!(settings.hotkey_conflict(HotkeyAction::CompletionAccept, binding)),
                fact("persisted evidence", observed == expected)
                    .details(format!("observed={observed:?}; expected={expected:?}"))
                    .expect(true),
            );
        };
    }

    let modified_up = HotkeyBinding::try_new(0x26, true, false, false, false).unwrap();
    {
        let observed = &(settings.hotkey_conflict(HotkeyAction::CompletionAccept, modified_up));
        let expected = &(None);
        c.check(
            stringify!(settings.hotkey_conflict(HotkeyAction::CompletionAccept, modified_up)),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn ctrl_break_canonicalizes_to_ctrl_pause_and_survives_reassignment() {
    let mut c = certification("ctrl_break_canonicalizes_to_ctrl_pause_and_survives_reassignment");
    let ctrl_break = HotkeyBinding::try_new(0x03, true, false, false, false).unwrap();
    {
        let observed = &(ctrl_break);
        let expected = &(HotkeyBinding::IGNORE_WORD_DEFAULT);
        c.check(
            stringify!(ctrl_break),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };

    {
        let observed = &(ctrl_break.as_stored());
        let expected = &("ctrl+vk:19");
        c.check(
            stringify!(ctrl_break.as_stored()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };

    let database = Database::open_in_memory().unwrap();
    let alternate = HotkeyBinding::try_new(b'F' as u16, true, false, false, false).unwrap();
    database
        .save_settings(AppSettings::default().with_hotkey(HotkeyAction::IgnoreWord, alternate))
        .unwrap();
    {
        let observed = &(database
            .settings()
            .unwrap()
            .hotkey(HotkeyAction::IgnoreWord));
        let expected = &(alternate);
        c.check(
            stringify!(
                database
                    .settings()
                    .unwrap()
                    .hotkey(HotkeyAction::IgnoreWord)
            ),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };

    database
        .save_settings(
            database
                .settings()
                .unwrap()
                .with_hotkey(HotkeyAction::IgnoreWord, ctrl_break),
        )
        .unwrap();
    {
        let observed = &(database
            .settings()
            .unwrap()
            .hotkey(HotkeyAction::IgnoreWord));
        let expected = &(HotkeyBinding::IGNORE_WORD_DEFAULT);
        c.check(
            stringify!(
                database
                    .settings()
                    .unwrap()
                    .hotkey(HotkeyAction::IgnoreWord)
            ),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn all_live_settings_round_trip_through_sqlite() {
    let mut c = certification("all_live_settings_round_trip_through_sqlite");
    let database = Database::open_in_memory().unwrap();
    let settings = AppSettings::default()
        .with_hotkey(
            HotkeyAction::ClipboardCurrent,
            HotkeyBinding::try_new(b'J' as u16, true, false, false, false).unwrap(),
        )
        .with_hotkey(
            HotkeyAction::CompletionAccept,
            HotkeyBinding::try_new(0x71, false, false, false, false).unwrap(),
        )
        .with_enable_autocomplete(false)
        .with_enable_autocorrections(false)
        .with_enable_auto_keyboard_switches(false)
        .with_notification_timeout_seconds(NotificationTimeoutSeconds::try_new(9).unwrap());

    database.save_settings(settings).unwrap();
    {
        let observed = &(database.settings().unwrap());
        let expected = &(settings);
        c.check(
            stringify!(database.settings().unwrap()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn clipboard_text_history_deduplicates_orders_filters_and_pins() {
    let mut c = certification("clipboard_text_history_deduplicates_orders_filters_and_pins");
    let database = Database::open_in_memory().unwrap();

    let hello = database.record_clipboard_text("Hello", 10).unwrap();
    let world = database.record_clipboard_text("world", 20).unwrap();
    let hello_again = database.record_clipboard_text("Hello", 30).unwrap();

    {
        let observed = &(hello_again.id());
        let expected = &(hello.id());
        c.check(
            stringify!(hello_again.id()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(hello_again.kind());
        let expected = &(ClipboardEntryKind::Text);
        c.check(
            stringify!(hello_again.kind()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(hello_again.copy_count());
        let expected = &(2);
        c.check(
            stringify!(hello_again.copy_count()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(hello_again.last_seen_at_ms());
        let expected = &(10);
        c.check(
            stringify!(hello_again.last_seen_at_ms()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(hello_again.content());
        let expected = &(&ClipboardEntryContent::Text("Hello".to_owned()));
        c.check(
            stringify!(hello_again.content()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };

    let current = database.load_clipboard_current("", 9).unwrap();
    {
        let observed = &(current.iter().map(|entry| entry.id()).collect::<Vec<_>>());
        let expected = &([hello.id(), world.id()]);
        c.check(
            stringify!(current.iter().map(|entry| entry.id()).collect::<Vec<_>>()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(database.load_clipboard_current("HEL", 9).unwrap().len());
        let expected = &(1);
        c.check(
            stringify!(database.load_clipboard_current("HEL", 9).unwrap().len()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };

    let pinned = database.pin_clipboard_entry(world.id(), 40).unwrap();
    {
        let observed = &(pinned.pinned_at_ms());
        let expected = &(Some(40));
        c.check(
            stringify!(pinned.pinned_at_ms()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(database.load_clipboard_pinned("wor", 9).unwrap()[0].id());
        let expected = &(world.id());
        c.check(
            stringify!(database.load_clipboard_pinned("wor", 9).unwrap()[0].id()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };

    database.mark_clipboard_entry_used(world.id(), 50).unwrap();
    {
        let observed = &(database.load_clipboard_current("", 9).unwrap()[0].id());
        let expected = &(world.id());
        c.check(
            stringify!(database.load_clipboard_current("", 9).unwrap()[0].id()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(database.load_clipboard_pinned("", 9).unwrap()[0].pinned_at_ms());
        let expected = &(Some(40));
        c.check(
            stringify!(database.load_clipboard_pinned("", 9).unwrap()[0].pinned_at_ms()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn clipboard_summary_query_returns_full_history_without_loading_image_payloads() {
    let mut c = certification(
        "clipboard_summary_query_returns_full_history_without_loading_image_payloads",
    );
    let database = Database::open_in_memory().unwrap();
    for index in 0..150 {
        database
            .record_clipboard_text(&format!("entry-{index:03}"), i64::from(index))
            .unwrap();
    }
    let image = database
        .record_clipboard_image("CF_DIB", &[1, 2, 3, 4], 500)
        .unwrap();

    let summaries = database.load_clipboard_current_summaries("").unwrap();
    {
        let observed = &(summaries.len());
        let expected = &(151);
        c.check(
            stringify!(summaries.len()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    let image_summary = summaries
        .iter()
        .find(|entry| entry.id() == image.id())
        .unwrap();
    {
        let observed = &(image_summary.content());
        let expected = &(&ClipboardEntrySummaryContent::Image {
            format: "CF_DIB".to_owned(),
        });
        c.check(
            stringify!(image_summary.content()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };

    let filtered = database
        .load_clipboard_current_summaries("ENTRY-149")
        .unwrap();
    {
        let observed = &(filtered.len());
        let expected = &(1);
        c.check(
            stringify!(filtered.len()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(filtered[0].content());
        let expected = &(&ClipboardEntrySummaryContent::Text("entry-149".to_owned()));
        c.check(
            stringify!(filtered[0].content()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn clipboard_current_entries_can_be_reordered_without_rewriting_entry_times() {
    let mut c =
        certification("clipboard_current_entries_can_be_reordered_without_rewriting_entry_times");
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
    {
        let observed = &(current.iter().map(|entry| entry.id()).collect::<Vec<_>>());
        let expected = &([first.id(), third.id(), second.id()]);
        c.check(
            stringify!(current.iter().map(|entry| entry.id()).collect::<Vec<_>>()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(current
            .iter()
            .map(|entry| entry.last_seen_at_ms())
            .collect::<Vec<_>>());
        let expected = &([10, 30, 20]);
        c.check(
            stringify!(
                current
                    .iter()
                    .map(|entry| entry.last_seen_at_ms())
                    .collect::<Vec<_>>()
            ),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };

    database
        .reorder_clipboard_entry(
            ClipboardEntryList::Current,
            second.id(),
            first.id(),
            ClipboardReorderPosition::After,
        )
        .unwrap();
    let current = database.load_clipboard_current("", 10).unwrap();
    {
        let observed = &(current.iter().map(|entry| entry.id()).collect::<Vec<_>>());
        let expected = &([first.id(), second.id(), third.id()]);
        c.check(
            stringify!(current.iter().map(|entry| entry.id()).collect::<Vec<_>>()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(current
            .iter()
            .map(|entry| entry.last_seen_at_ms())
            .collect::<Vec<_>>());
        let expected = &([10, 20, 30]);
        c.check(
            stringify!(
                current
                    .iter()
                    .map(|entry| entry.last_seen_at_ms())
                    .collect::<Vec<_>>()
            ),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn clipboard_pinned_entries_can_be_reordered_without_rewriting_pin_times() {
    let mut c =
        certification("clipboard_pinned_entries_can_be_reordered_without_rewriting_pin_times");
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
    {
        let observed = &(pinned.iter().map(|entry| entry.id()).collect::<Vec<_>>());
        let expected = &([first.id(), third.id(), second.id()]);
        c.check(
            stringify!(pinned.iter().map(|entry| entry.id()).collect::<Vec<_>>()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(pinned
            .iter()
            .map(|entry| entry.pinned_at_ms())
            .collect::<Vec<_>>());
        let expected = &([Some(40), Some(60), Some(50)]);
        c.check(
            stringify!(
                pinned
                    .iter()
                    .map(|entry| entry.pinned_at_ms())
                    .collect::<Vec<_>>()
            ),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}
#[test]
fn clipboard_image_history_is_loaded_with_text_entries() {
    let mut c = certification("clipboard_image_history_is_loaded_with_text_entries");
    let database = Database::open_in_memory().unwrap();
    let text = database.record_clipboard_text("hello", 10).unwrap();
    let image = database
        .record_clipboard_image("CF_DIB", &[1, 2, 3, 4], 20)
        .unwrap();

    {
        let observed = &(image.kind());
        let expected = &(ClipboardEntryKind::Image);
        c.check(
            stringify!(image.kind()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(image.content());
        let expected = &(&ClipboardEntryContent::Image {
            format: "CF_DIB".to_owned(),
            data: vec![1, 2, 3, 4],
        });
        c.check(
            stringify!(image.content()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(database
            .load_clipboard_current("image", 10)
            .unwrap()
            .iter()
            .map(|entry| entry.id())
            .collect::<Vec<_>>());
        let expected = &([image.id()]);
        c.check(
            stringify!(
                database
                    .load_clipboard_current("image", 10)
                    .unwrap()
                    .iter()
                    .map(|entry| entry.id())
                    .collect::<Vec<_>>()
            ),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(database
            .load_clipboard_current("", 10)
            .unwrap()
            .iter()
            .map(|entry| entry.id())
            .collect::<Vec<_>>());
        let expected = &([image.id(), text.id()]);
        c.check(
            stringify!(
                database
                    .load_clipboard_current("", 10)
                    .unwrap()
                    .iter()
                    .map(|entry| entry.id())
                    .collect::<Vec<_>>()
            ),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn clipboard_prune_keeps_pinned_entries_and_recent_current_entries() {
    let mut c = certification("clipboard_prune_keeps_pinned_entries_and_recent_current_entries");
    let database = Database::open_in_memory().unwrap();
    let old = database.record_clipboard_text("old", 10).unwrap();
    let pinned = database.record_clipboard_text("pinned", 20).unwrap();
    let recent = database.record_clipboard_text("recent", 30).unwrap();
    database.pin_clipboard_entry(pinned.id(), 40).unwrap();

    {
        let observed = &(database
            .prune_clipboard_history(ClipboardHistoryLimit::try_new(1).unwrap())
            .unwrap());
        let expected = &(1);
        c.check(
            stringify!(
                database
                    .prune_clipboard_history(ClipboardHistoryLimit::try_new(1).unwrap())
                    .unwrap()
            ),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.check(
        stringify!(matches!(
            database.mark_clipboard_entry_used(old.id(), 50),
            Err(DatabaseError::ClipboardEntryNotFound(_))
        )),
        fact!(matches!(
            database.mark_clipboard_entry_used(old.id(), 50),
            Err(DatabaseError::ClipboardEntryNotFound(_))
        ))
        .expect(true),
    );
    {
        let observed = &(database
            .load_clipboard_current("", 10)
            .unwrap()
            .iter()
            .map(|entry| entry.id())
            .collect::<Vec<_>>());
        let expected = &([recent.id(), pinned.id()]);
        c.check(
            stringify!(
                database
                    .load_clipboard_current("", 10)
                    .unwrap()
                    .iter()
                    .map(|entry| entry.id())
                    .collect::<Vec<_>>()
            ),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn clipboard_clear_all_removes_only_clipboard_storage() {
    let mut c = certification("clipboard_clear_all_removes_only_clipboard_storage");
    let database = Database::open_in_memory().unwrap();
    let text = database
        .record_clipboard_text("learned clipboard text", 10)
        .unwrap();
    let image = database
        .record_clipboard_image("CF_DIB", &[1, 2, 3, 4], 20)
        .unwrap();
    database.pin_clipboard_entry(image.id(), 30).unwrap();
    database
        .record_user_word("LearnedClipboardToken", 40)
        .unwrap();
    database
        .record_text_history("learned clipboard", 50)
        .unwrap();

    {
        let observed = &(database.clear_clipboard_history().unwrap());
        let expected = &(2);
        c.check(
            stringify!(database.clear_clipboard_history().unwrap()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.check(
        stringify!(database.load_clipboard_current("", 10).unwrap().is_empty()),
        fact!(database.load_clipboard_current("", 10).unwrap().is_empty()).expect(true),
    );
    c.check(
        stringify!(database.load_clipboard_pinned("", 10).unwrap().is_empty()),
        fact!(database.load_clipboard_pinned("", 10).unwrap().is_empty()).expect(true),
    );
    c.check(
        stringify!(matches!(
            database.mark_clipboard_entry_used(text.id(), 60),
            Err(DatabaseError::ClipboardEntryNotFound(_))
        )),
        fact!(matches!(
            database.mark_clipboard_entry_used(text.id(), 60),
            Err(DatabaseError::ClipboardEntryNotFound(_))
        ))
        .expect(true),
    );
    c.check(
        stringify!(database.delete_user_word("learnedclipboardtoken").unwrap()),
        fact!(database.delete_user_word("learnedclipboardtoken").unwrap()).expect(true),
    );
    c.check(
        stringify!(database.delete_text_history("learned clipboard").unwrap()),
        fact!(database.delete_text_history("learned clipboard").unwrap()).expect(true),
    );

    let rerecorded = database
        .record_clipboard_image("CF_DIB", &[1, 2, 3, 4], 70)
        .unwrap();
    {
        let observed = &(rerecorded.copy_count());
        let expected = &(1);
        c.check(
            stringify!(rerecorded.copy_count()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn user_word_upsert_accumulates_usage_without_rewriting_canonical_spelling() {
    let mut c =
        certification("user_word_upsert_accumulates_usage_without_rewriting_canonical_spelling");
    let database = Database::open_in_memory().unwrap();
    database
        .record_user_word("QuantileEntryStrategy1", 200)
        .unwrap();
    let older = database
        .record_user_word("quantileentrystrategy1", 150)
        .unwrap();
    {
        let observed = &(older.term());
        let expected = &("QuantileEntryStrategy1");
        c.check(
            stringify!(older.term()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(older.use_count());
        let expected = &(2);
        c.check(
            stringify!(older.use_count()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(older.last_used_at_ms());
        let expected = &(200);
        c.check(
            stringify!(older.last_used_at_ms()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };

    let newer = database
        .record_user_word("quantileentrystrategy1", 250)
        .unwrap();
    {
        let observed = &(newer.term());
        let expected = &("QuantileEntryStrategy1");
        c.check(
            stringify!(newer.term()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(newer.use_count());
        let expected = &(3);
        c.check(
            stringify!(newer.use_count()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(newer.last_used_at_ms());
        let expected = &(250);
        c.check(
            stringify!(newer.last_used_at_ms()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn correction_undo_requires_valid_text_is_single_use_and_adds_a_user_word() {
    let mut c =
        certification("correction_undo_requires_valid_text_is_single_use_and_adds_a_user_word");
    let mut database = Database::open_in_memory().unwrap();
    c.check(
        stringify!(matches!(
            database.record_correction_event("", "fixed", 1),
            Err(DatabaseError::InvalidCorrectionText)
        )),
        fact!(matches!(
            database.record_correction_event("", "fixed", 1),
            Err(DatabaseError::InvalidCorrectionText)
        ))
        .expect(true),
    );

    database.ignore_word("QuantileEntryStrategy1").unwrap();
    let event = database
        .record_correction_event("QuantileEntryStrategy1", "QuantileEntryStrategy", 10)
        .unwrap();
    let plan = database.prepare_correction_undo(event).unwrap();
    {
        let observed = &(plan.original_text());
        let expected = &("QuantileEntryStrategy1");
        c.check(
            stringify!(plan.original_text()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(plan.replacement_text());
        let expected = &("QuantileEntryStrategy");
        c.check(
            stringify!(plan.replacement_text()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };

    let learned = database.commit_correction_undo(event, 20).unwrap();
    {
        let observed = &(learned.term());
        let expected = &("QuantileEntryStrategy1");
        c.check(
            stringify!(learned.term()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    {
        let observed = &(learned.use_count());
        let expected = &(1);
        c.check(
            stringify!(learned.use_count()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.check(
        stringify!(
            !database
                .load_ignored_words()
                .unwrap()
                .contains_normalized("quantileentrystrategy1")
        ),
        fact!(
            !database
                .load_ignored_words()
                .unwrap()
                .contains_normalized("quantileentrystrategy1")
        )
        .expect(true),
    );
    c.check(
        stringify!(matches!(
            database.prepare_correction_undo(event),
            Err(DatabaseError::CorrectionEventAlreadyUndone(_))
        )),
        fact!(matches!(
            database.prepare_correction_undo(event),
            Err(DatabaseError::CorrectionEventAlreadyUndone(_))
        ))
        .expect(true),
    );
    c.assert_certify();
}

#[test]
fn user_word_delete_is_normalized_and_idempotent() {
    let mut c = certification("user_word_delete_is_normalized_and_idempotent");
    let database = Database::open_in_memory().unwrap();
    database
        .record_user_word("QuantileEntryStrategy1", 1)
        .unwrap();
    database.record_user_word("OtherToken", 2).unwrap();

    c.check(
        stringify!(database.delete_user_word("QUANTILEENTRYSTRATEGY1").unwrap()),
        fact!(database.delete_user_word("QUANTILEENTRYSTRATEGY1").unwrap()).expect(true),
    );
    c.check(
        stringify!(!database.delete_user_word("quantileentrystrategy1").unwrap()),
        fact!(!database.delete_user_word("quantileentrystrategy1").unwrap()).expect(true),
    );

    let lexicon = database.load_user_lexicon().unwrap();
    c.check(
        stringify!(!lexicon.contains_normalized("quantileentrystrategy1")),
        fact!(!lexicon.contains_normalized("quantileentrystrategy1")).expect(true),
    );
    c.check(
        stringify!(lexicon.contains_normalized("othertoken")),
        fact!(lexicon.contains_normalized("othertoken")).expect(true),
    );
    c.assert_certify();
}

#[test]
fn ignored_word_addition_removes_user_word_and_forget_resets_the_override() {
    let mut c =
        certification("ignored_word_addition_removes_user_word_and_forget_resets_the_override");
    let mut database = Database::open_in_memory().unwrap();
    database.record_user_word("JR", 1).unwrap();

    {
        let observed = &(database.ignore_word("jr").unwrap());
        let expected = &(IgnoreWordOutcome::AddedToIgnoreList);
        c.check(
            stringify!(database.ignore_word("jr").unwrap()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.check(
        stringify!(
            !database
                .load_user_lexicon()
                .unwrap()
                .contains_normalized("jr")
        ),
        fact!(
            !database
                .load_user_lexicon()
                .unwrap()
                .contains_normalized("jr")
        )
        .expect(true),
    );
    c.check(
        stringify!(
            database
                .load_ignored_words()
                .unwrap()
                .contains_normalized("jr")
        ),
        fact!(
            database
                .load_ignored_words()
                .unwrap()
                .contains_normalized("jr")
        )
        .expect(true),
    );
    {
        let observed = &(database.ignore_word("JR").unwrap());
        let expected = &(IgnoreWordOutcome::AlreadyInIgnoreList);
        c.check(
            stringify!(database.ignore_word("JR").unwrap()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };

    {
        let observed = &(database.forget_word("JR").unwrap());
        let expected = &(ForgetWordOutcome::RemovedFromIgnoreList);
        c.check(
            stringify!(database.forget_word("JR").unwrap()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.check(
        stringify!(
            !database
                .load_ignored_words()
                .unwrap()
                .contains_normalized("jr")
        ),
        fact!(
            !database
                .load_ignored_words()
                .unwrap()
                .contains_normalized("jr")
        )
        .expect(true),
    );
    {
        let observed = &(database.forget_word("jr").unwrap());
        let expected = &(ForgetWordOutcome::NotFound);
        c.check(
            stringify!(database.forget_word("jr").unwrap()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

#[test]
fn active_text_history_preserves_strong_repetition_and_fresh_singletons_without_deleting_rows() {
    let mut c = certification(
        "active_text_history_preserves_strong_repetition_and_fresh_singletons_without_deleting_rows",
    );
    let database = Database::open_in_memory().unwrap();

    database.record_text_history("repeat strong", 10).unwrap();
    database.record_text_history("repeat strong", 20).unwrap();
    database.record_text_history("repeat strong", 30).unwrap();

    database.record_text_history("repeat weaker", 100).unwrap();
    database.record_text_history("repeat weaker", 110).unwrap();

    database.record_text_history("singleton old", 200).unwrap();
    database
        .record_text_history("singleton fresh", 300)
        .unwrap();

    let active = database.load_active_text_history(1, 1).unwrap();
    let mut active_texts = active
        .entries()
        .iter()
        .map(|entry| entry.text().to_owned())
        .collect::<Vec<_>>();
    active_texts.sort();
    {
        let observed = &(active_texts);
        let expected = &(["repeat strong", "singleton fresh"]);
        c.check(
            stringify!(active_texts),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };

    let full = database.load_text_history().unwrap();
    {
        let observed = &(full.entries().len());
        let expected = &(4);
        c.check(
            stringify!(full.entries().len()),
            fact("persisted evidence", observed == expected)
                .details(format!("observed={observed:?}; expected={expected:?}"))
                .expect(true),
        );
    };
    c.assert_certify();
}

fn storage_pragmas(database: &Database) -> rusqlite::Result<(String, i64)> {
    Ok((
        database
            .connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))?,
        database
            .connection
            .pragma_query_value(None, "synchronous", |row| row.get(0))?,
    ))
}

const RETENTION_NOW_MS: i64 = 1_736_899_200_123;
const RETENTION_CUTOFFS_MS: [i64; 12] = [
    1_734_220_800_123,
    1_731_628_800_123,
    1_728_950_400_123,
    1_726_358_400_123,
    1_723_680_000_123,
    1_721_001_600_123,
    1_718_409_600_123,
    1_715_731_200_123,
    1_713_139_200_123,
    1_710_460_800_123,
    1_707_955_200_123,
    1_705_276_800_123,
];

#[derive(Clone, Debug, DomainInput)]
struct RetentionInput {
    use_count: u32,
    last_used_at_ms: i64,
}

#[test]
fn certification_sequence_inactivity_retention_family_preserves_survivor_metadata() {
    let mut runner = certification("sequence retention follows capped use count and last use")
        .executor(|input: &RetentionInput| -> Result<_, DatabaseError> {
            let mut database = Database::open_in_memory()?;

            database.connection.execute(
                "INSERT INTO text_history(text, use_count, last_used_at_ms) VALUES ('hello world', ?1, ?2)",
                params![input.use_count, input.last_used_at_ms],
            )?;
            let report = database.maintain(RETENTION_NOW_MS)?;
            let history = database.load_text_history()?;
            let rows = history.entries().iter().map(|r| (r.use_count(), r.last_used_at_ms())).collect::<Vec<_>>();
            let repeated = database.maintain(RETENTION_NOW_MS)?;
            Ok((report, rows, repeated))
        });
    runner.domain(domain!(RetentionInput {
        use_count.in_range(1..=u32::MAX),
        last_used_at_ms.in_range(i64::MIN..=i64::MAX),
    }));
    runner.data_source([
        RetentionInput {
            use_count: 1,
            last_used_at_ms: RETENTION_CUTOFFS_MS[0] - 1,
        },
        RetentionInput {
            use_count: 1,
            last_used_at_ms: RETENTION_CUTOFFS_MS[0],
        },
        RetentionInput {
            use_count: 1,
            last_used_at_ms: RETENTION_CUTOFFS_MS[0] + 1,
        },
        RetentionInput {
            use_count: 2,
            last_used_at_ms: RETENTION_CUTOFFS_MS[1] - 1,
        },
        RetentionInput {
            use_count: 2,
            last_used_at_ms: RETENTION_CUTOFFS_MS[1] + 15 * 86_400_000,
        },
        RetentionInput {
            use_count: 12,
            last_used_at_ms: RETENTION_CUTOFFS_MS[11] + 1,
        },
        RetentionInput {
            use_count: 100_000,
            last_used_at_ms: RETENTION_CUTOFFS_MS[11] - 1,
        },
    ]);
    runner.cases_budget(160).seed(741);
    let case = runner.case();
    runner.check(
        fact!(case.result().0.text_history_deleted).expect(fact!(usize::from(
            case.input().last_used_at_ms
                < RETENTION_CUTOFFS_MS[(case.input().use_count.min(12) - 1) as usize]
        ))),
    );
    runner.check(fact!(case.result().0.clipboard_entries_deleted).expect(0usize));
    runner.check(fact!(case.result().1.clone()).expect(fact!({
        let input = case.input();
        if input.last_used_at_ms < RETENTION_CUTOFFS_MS[(input.use_count.min(12) - 1) as usize] {
            Vec::new()
        } else {
            vec![(input.use_count, input.last_used_at_ms)]
        }
    })));
    runner.check(fact!(case.result().2).expect(MaintenanceReport {
        text_history_deleted: 0,
        clipboard_entries_deleted: 0,
    }));
    runner.assert_certify();
}

#[test]
fn certification_calendar_month_end_and_negative_epoch_keep_millisecond_boundaries() {
    let mut runner = certification("UTC calendar month-end clamps without losing milliseconds")
        .executor(|&(now, cutoff): &(i64, i64)| -> Result<_, DatabaseError> {
            let mut database = Database::open_in_memory()?;
            database.record_text_history("just expired", cutoff - 1)?;
            database.record_text_history("exact boundary", cutoff)?;
            database.record_text_history("still fresh", cutoff + 1)?;
            database.record_text_history("future evidence", now + 1)?;
            let report = database.maintain(now)?;
            let mut rows = database
                .load_text_history()?
                .entries()
                .iter()
                .map(|r| (r.text().to_owned(), r.use_count(), r.last_used_at_ms()))
                .collect::<Vec<_>>();
            rows.sort();
            Ok((report, rows))
        });
    runner.data_source([
        (1_711_843_200_123, 1_709_164_800_123),
        (1_743_379_200_123, 1_740_700_800_123),
        (-86_399_877, -2_764_799_877),
    ]);
    let case = runner.case();
    runner.check(fact!(case.result().0.text_history_deleted).expect(1usize));
    runner.check(fact!(case.result().1.clone()).expect(fact!({
        let input = case.input();
        vec![
            ("exact boundary".to_owned(), 1u32, input.1),
            ("future evidence".to_owned(), 1u32, input.0 + 1),
            ("still fresh".to_owned(), 1u32, input.1 + 1),
        ]
    })));
    runner.assert_certify();
}

#[derive(Clone, Copy, Debug, DomainInput)]
enum PayloadKind {
    Text,
    Image,
    Files,
}

fn insert_payload(
    connection: &Connection,
    id: i64,
    kind: PayloadKind,
    bytes: i64,
    pinned: bool,
    order: i64,
) {
    let kind_name = match kind {
        PayloadKind::Text => "text",
        PayloadKind::Image => "image",
        PayloadKind::Files => "files",
    };
    connection.execute(
        "INSERT INTO clipboard_entries(id, kind, content_hash, last_seen_at_ms, current_order_index, pinned_at_ms) VALUES (?1, ?2, CAST(?1 AS BLOB), -?1, ?3, ?4)",
        params![id, kind_name, order, if pinned { Some(1) } else { None::<i64> }],
    ).unwrap();
    match kind {
        PayloadKind::Text => connection.execute(
            "INSERT INTO clipboard_text VALUES (?1, printf('%*s', ?2, 'x'))",
            params![id, bytes],
        ),
        PayloadKind::Image => connection.execute(
            "INSERT INTO clipboard_images VALUES (?1, 'dib', zeroblob(?2))",
            params![id, bytes - 3],
        ),
        PayloadKind::Files => connection.execute(
            "INSERT INTO clipboard_files VALUES (?1, 0, printf('%*s', ?2, 'x'))",
            params![id, bytes],
        ),
    }
    .unwrap();
}

fn persisted_clipboard_ids(database: &Database) -> Vec<i64> {
    database
        .connection
        .prepare("SELECT id FROM clipboard_entries ORDER BY current_order_index DESC, id DESC")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[derive(Clone, Debug, DomainInput)]
struct ClipboardCountInput {
    entries: u8,
    limit: u32,
    pinned_first: bool,
    tied_order: bool,
    kind: PayloadKind,
}

#[test]
fn certification_clipboard_count_and_order_family_excludes_pinned_and_cascades_children() {
    let mut runner = certification("count pruning uses current order and never counts pinned payload")
        .executor(|input: &ClipboardCountInput| -> Result<_, DatabaseError> {
            let database = Database::open_in_memory()?;
            let limit = ClipboardHistoryLimit::try_new(input.limit)
                .map_err(|_| DatabaseError::InvalidStoredClipboardHistoryLimit(input.limit.into()))?;
            for id in 1..=input.entries {
                insert_payload(&database.connection, id.into(), input.kind, 16, input.pinned_first && id == 1,
                    if input.tied_order { 0 } else { -i64::from(id) });
            }
            let mut expected = Vec::new();
            let ids: Vec<_> = if input.tied_order { (1..=input.entries).rev().collect() } else { (1..=input.entries).collect() };
            let mut unpinned_kept = 0;
            for id in ids {
                if input.pinned_first && id == 1 { expected.push(i64::from(id)); }
                else if unpinned_kept < input.limit { expected.push(i64::from(id)); unpinned_kept += 1; }
            }
            let deleted = database.prune_clipboard_history(limit)?;
            let actual = persisted_clipboard_ids(&database);
            let children: i64 = database.connection.query_row("SELECT (SELECT count(*) FROM clipboard_text) + (SELECT count(*) FROM clipboard_images) + (SELECT count(*) FROM clipboard_files)", [], |r| r.get(0))?;
            Ok((deleted, actual, expected, children))
        });
    runner.domain(domain!(ClipboardCountInput {
        entries.in_range(u8::MIN..=u8::MAX), limit.in_range(1..=u32::MAX),
        pinned_first.selected_values([false, true]), tied_order.selected_values([false, true]), kind.all_enum_values(),
    }));
    runner.cases_budget(100).seed(119);
    let case = runner.case();
    runner.check(fact!(case.result().1.clone()).expect(fact!(case.result().2.clone())));
    runner.check(
        fact!(case.result().0).expect(fact!(case.input().entries as usize - case.result().1.len())),
    );
    runner.check(fact!(case.result().3).expect(fact!(case.result().1.len() as i64)));
    runner.assert_certify();
}

#[test]
fn certification_clipboard_mixed_payload_budget_prunes_multiple_oldest_and_keeps_oversized_pin() {
    let mut c = certification("mixed UTF-8 text image and file bytes share one unpinned budget");
    let path = TempDatabasePath::new("mixed-payload-budget");
    let database = Database::open(path.as_path()).unwrap();
    const MIB: i64 = 1024 * 1024;
    insert_payload(
        &database.connection,
        1,
        PayloadKind::Image,
        20 * MIB,
        false,
        10,
    );
    insert_payload(
        &database.connection,
        2,
        PayloadKind::Text,
        20 * MIB,
        false,
        20,
    );
    database.connection.execute("UPDATE clipboard_text SET text = replace(printf('%*s', ?1, ''), ' ', 'я') WHERE entry_id = 2", [10 * MIB]).unwrap();
    insert_payload(
        &database.connection,
        3,
        PayloadKind::Files,
        20 * MIB,
        false,
        30,
    );
    insert_payload(
        &database.connection,
        4,
        PayloadKind::Image,
        170 * MIB,
        false,
        40,
    );
    insert_payload(
        &database.connection,
        5,
        PayloadKind::Image,
        210 * MIB,
        true,
        0,
    );
    c.check_fact_expect(
        "byte pressure deletes two oldest despite five entries",
        database
            .prune_clipboard_history(ClipboardHistoryLimit::DEFAULT)
            .unwrap(),
        2,
    );
    c.check_fact_expect(
        "files newest image and oversized pinned survive",
        persisted_clipboard_ids(&database),
        vec![4, 3, 5],
    );
    let stored: i64 = database.connection.query_row("SELECT (SELECT sum(length(data) + length(CAST(format AS BLOB))) FROM clipboard_images WHERE entry_id = 4) + (SELECT sum(length(CAST(path AS BLOB))) FROM clipboard_files)", [], |r| r.get(0)).unwrap();
    c.check_fact_expect("retained unpinned payload", stored, 190 * MIB);
    c.check_fact_expect(
        "removed text child cascades",
        database
            .connection
            .query_row("SELECT count(*) FROM clipboard_text", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
        0,
    );
    c.check_fact_expect(
        "removed image child cascades",
        database
            .connection
            .query_row("SELECT count(*) FROM clipboard_images", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
        2,
    );
    c.check_fact_expect(
        "repeat is idempotent",
        database
            .prune_clipboard_history(ClipboardHistoryLimit::DEFAULT)
            .unwrap(),
        0,
    );
    database
        .connection
        .execute(
            "UPDATE clipboard_entries SET current_order_index = 50 WHERE id = 3",
            [],
        )
        .unwrap();
    c.check_fact_expect(
        "count still applies after byte pruning",
        database
            .prune_clipboard_history(ClipboardHistoryLimit::try_new(1).unwrap())
            .unwrap(),
        1,
    );
    c.check_fact_expect(
        "current order not original timestamp chooses survivor",
        persisted_clipboard_ids(&database),
        vec![3, 5],
    );
    c.assert_certify();
}

#[test]
fn certification_clipboard_exact_budget_and_oversized_newest_have_no_byte_escape() {
    let mut c = certification("exact 200 MiB survives but one additional payload byte expires");
    let path = TempDatabasePath::new("exact-payload-budget");
    let database = Database::open(path.as_path()).unwrap();
    insert_payload(
        &database.connection,
        1,
        PayloadKind::Image,
        200 * 1024 * 1024,
        false,
        10,
    );
    c.check_fact_expect(
        "exact inclusive byte budget",
        database
            .prune_clipboard_history(ClipboardHistoryLimit::DEFAULT)
            .unwrap(),
        0,
    );
    database
        .connection
        .execute(
            "UPDATE clipboard_images SET data = zeroblob(?1)",
            [200 * 1024 * 1024 - 2],
        )
        .unwrap();
    c.check_fact_expect(
        "format bytes count and oversized newest is removed",
        database
            .prune_clipboard_history(ClipboardHistoryLimit::DEFAULT)
            .unwrap(),
        1,
    );
    c.check_fact_expect(
        "parent and payload removed",
        persisted_clipboard_ids(&database).is_empty(),
        true,
    );
    c.assert_certify();
}

#[test]
fn certification_maintenance_failure_rolls_back_both_cleanup_stores_and_preserves_user_state() {
    let mut c = certification("maintenance commits both cleanups atomically or neither");
    let mut database = Database::open_in_memory().unwrap();
    database.record_text_history("stale evidence", 10).unwrap();
    database.record_user_word("customword", 11).unwrap();
    database.ignore_word("ignoredword").unwrap();
    database.hide_completion_word("hiddenword").unwrap();
    let settings = AppSettings::new(
        ClipboardHistoryLimit::try_new(1).unwrap(),
        UndoHotkey::Pause,
    );
    database.save_settings(settings).unwrap();
    database.record_clipboard_text("old", 1).unwrap();
    database.record_clipboard_text("new", 2).unwrap();
    database.connection.execute_batch("CREATE TRIGGER block_prune BEFORE DELETE ON clipboard_entries BEGIN SELECT RAISE(ABORT, 'blocked'); END;").unwrap();
    c.check_fact_expect(
        "failed maintenance returns SQLite rejection",
        matches!(
            database.maintain(RETENTION_NOW_MS),
            Err(DatabaseError::Sqlite(_))
        ),
        true,
    );
    c.check_fact_expect(
        "sequence deletion rolled back",
        database.load_text_history().unwrap().entries().len(),
        1,
    );
    c.check_fact_expect(
        "clipboard deletion rolled back",
        persisted_clipboard_ids(&database).len(),
        2,
    );
    c.check_fact_expect(
        "invalid timestamp fails closed",
        matches!(
            database.maintain(i64::MAX),
            Err(DatabaseError::InvalidMaintenanceTimestamp(i64::MAX))
        ),
        true,
    );
    database
        .connection
        .execute_batch("DROP TRIGGER block_prune")
        .unwrap();
    c.check_fact_expect(
        "retry commits",
        database.maintain(RETENTION_NOW_MS).unwrap(),
        MaintenanceReport {
            text_history_deleted: 1,
            clipboard_entries_deleted: 1,
        },
    );
    c.check_fact_expect(
        "user word unchanged",
        database
            .load_user_lexicon()
            .unwrap()
            .exact("customword")
            .map(|r| (r.use_count(), r.last_used_at_ms())),
        Some((1, 11)),
    );
    c.check_fact_expect(
        "ignored survives",
        database
            .load_ignored_words()
            .unwrap()
            .contains_normalized("ignoredword"),
        true,
    );
    c.check_fact_expect(
        "suppression survives",
        database
            .load_hidden_completion_words()
            .unwrap()
            .contains("hiddenword"),
        true,
    );
    c.check_fact_expect("settings survive", database.settings().unwrap(), settings);
    c.assert_certify();
}

#[test]
fn certification_v9_retention_migration_is_indexed_idempotent_and_non_destructive() {
    let mut c = certification("v9 upgrades retention access without rewriting evidence");
    let path = TempDatabasePath::new("retention-migration");
    {
        let database = Database::open(path.as_path()).unwrap();
        database.record_text_history("old evidence", 10).unwrap();
        database
            .connection
            .execute_batch("DROP INDEX text_history_retention_idx; PRAGMA user_version = 9;")
            .unwrap();
    }
    let mut database = Database::open_existing(path.as_path()).unwrap();
    c.check_fact_expect("version upgraded", database.schema_version().unwrap(), 10);
    c.check_fact_expect(
        "migration does not expire until explicit maintenance",
        database.load_text_history().unwrap().entries().len(),
        1,
    );
    let plan: String = database.connection.query_row("EXPLAIN QUERY PLAN DELETE FROM text_history WHERE min(use_count, 12) = 1 AND last_used_at_ms < 100", [], |r| r.get(3)).unwrap();
    c.check_fact_expect(
        "retention range uses index",
        plan.contains("SEARCH text_history USING INDEX text_history_retention_idx"),
        true,
    );
    database.maintain(RETENTION_NOW_MS).unwrap();
    database.vacuum_exclusive().unwrap();
    drop(database);
    let reopened = Database::open_existing(path.as_path()).unwrap();
    c.check_fact_expect(
        "vacuum preserves valid database",
        reopened
            .connection
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok".to_owned(),
    );
    c.check_fact_expect(
        "vacuum reopen retains cleanup",
        reopened.load_text_history().unwrap().entries().len(),
        0,
    );
    c.assert_certify();
}
