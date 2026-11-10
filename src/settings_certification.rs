use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use sun_observations::certifications::*;

use super::*;

struct Fixture {
    store: SettingsStore,
    path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sunswitcher-settings-{}-{nonce}.db",
            std::process::id()
        ));
        let store = SettingsStore::from_database(Database::open(&path).unwrap()).unwrap();
        Self { store, path }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.store.writer.lock().unwrap().take();
        for path in [
            &self.path,
            &self.path.with_extension("db-wal"),
            &self.path.with_extension("db-shm"),
        ] {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[test]
fn live_application_changes_preserve_globals_and_rehydrate_from_canonical_persistence() {
    let mut c = certification("live application suppression and inheritance");
    let fixture = Fixture::new();
    let store = &fixture.store;
    let application = ApplicationIdentity::try_new(r"C:\Apps\example.exe").unwrap();
    let ordinary = ApplicationIdentity::try_new(r"D:\Apps\example.exe").unwrap();
    store
        .set_application_suppressed(&application, true)
        .unwrap();
    c.check_fact_expect(
        "active suppression",
        store.correction_policy(Some(&application)),
        CorrectionFeaturePolicy::new(false, false),
    );
    c.check_fact_expect(
        "switching to ordinary inherits",
        store.correction_policy(Some(&ordinary)),
        CorrectionFeaturePolicy::ALL,
    );
    c.check_fact_expect(
        "switching back suppresses again",
        store.correction_policy(Some(&application)),
        CorrectionFeaturePolicy::new(false, false),
    );
    c.check_fact_expect(
        "globals unchanged",
        store.load().unwrap(),
        AppSettings::default(),
    );
    {
        let reopened =
            SettingsStore::from_database(Database::open(&fixture.path).unwrap()).unwrap();
        c.check_fact_expect(
            "suppression survives restart",
            reopened.correction_policy(Some(&application)),
            CorrectionFeaturePolicy::new(false, false),
        );
        c.check_fact_expect(
            "restart agrees with live projection",
            reopened.application_overrides().unwrap(),
            store.application_overrides().unwrap(),
        );
    }
    store
        .update(|current| current.with_enable_autocorrections(false))
        .unwrap();
    store
        .set_application_suppressed(&application, false)
        .unwrap();
    c.check_fact_expect(
        "removal inherits global OFF without changing layout authority",
        store.correction_policy(Some(&application)),
        CorrectionFeaturePolicy::new(false, true),
    );
    c.check_fact_expect(
        "autocomplete remains enabled",
        store.load().unwrap().enable_autocomplete(),
        true,
    );
    let reopened = SettingsStore::from_database(Database::open(&fixture.path).unwrap()).unwrap();
    c.check_fact_expect(
        "global and removal persist",
        reopened.load().unwrap(),
        store.load().unwrap(),
    );
    c.check_fact_expect(
        "removed suppression remains absent",
        reopened
            .application_overrides()
            .unwrap()
            .suppresses(&application),
        false,
    );
    c.assert_certify();
}

#[test]
fn failed_global_and_application_writes_do_not_publish_success() {
    let mut c = certification("failed settings persistence leaves projections unchanged");
    let fixture = Fixture::new();
    let store = &fixture.store;
    let raw = Connection::open(&fixture.path).unwrap();
    let application = ApplicationIdentity::try_new(r"C:\Apps\example.exe").unwrap();
    let initial = store.load().unwrap();
    raw.execute_batch("CREATE TRIGGER reject_settings BEFORE UPDATE ON app_settings BEGIN SELECT RAISE(FAIL, 'test write failure'); END;
        CREATE TRIGGER reject_application BEFORE INSERT ON application_replacement_suppressions BEGIN SELECT RAISE(FAIL, 'test write failure'); END;").unwrap();
    c.check_fact_expect(
        "global write reports SQLite rejection",
        matches!(
            store.update(|current| current.with_enable_autocorrections(false)),
            Err(SettingsWriteError::Database(DatabaseError::Sqlite(_)))
        ),
        true,
    );
    c.check_fact_expect(
        "global live value unchanged",
        store.load().unwrap(),
        initial,
    );
    c.check_fact_expect(
        "global canonical value unchanged",
        Database::open(&fixture.path).unwrap().settings().unwrap(),
        initial,
    );
    c.check_fact_expect(
        "application insertion reports SQLite rejection",
        matches!(
            store.set_application_suppressed(&application, true),
            Err(SettingsWriteError::Database(DatabaseError::Sqlite(_)))
        ),
        true,
    );
    c.check_fact_expect(
        "failed insertion absent from projection",
        store
            .application_overrides()
            .unwrap()
            .suppresses(&application),
        false,
    );
    c.check_fact_expect(
        "failed insertion absent from persistence",
        Database::open(&fixture.path)
            .unwrap()
            .load_application_overrides()
            .unwrap()
            .suppresses(&application),
        false,
    );
    raw.execute_batch("DROP TRIGGER reject_application;")
        .unwrap();
    store
        .set_application_suppressed(&application, true)
        .unwrap();
    raw.execute_batch("CREATE TRIGGER reject_removal BEFORE DELETE ON application_replacement_suppressions BEGIN SELECT RAISE(FAIL, 'test write failure'); END;").unwrap();
    c.check_fact_expect(
        "removal reports SQLite rejection",
        matches!(
            store.set_application_suppressed(&application, false),
            Err(SettingsWriteError::Database(DatabaseError::Sqlite(_)))
        ),
        true,
    );
    c.check_fact_expect(
        "failed removal stays suppressed in projection",
        store
            .application_overrides()
            .unwrap()
            .suppresses(&application),
        true,
    );
    c.check_fact_expect(
        "failed removal stays suppressed in persistence",
        Database::open(&fixture.path)
            .unwrap()
            .load_application_overrides()
            .unwrap()
            .suppresses(&application),
        true,
    );
    c.assert_certify();
}

#[test]
fn shared_writer_merges_simultaneous_changes_from_latest_state() {
    let mut c = certification("concurrent Settings and tray changes do not overwrite each other");
    let fixture = Fixture::new();
    let ui = fixture.store.clone();
    let tray = fixture.store.clone();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let ui_barrier = Arc::clone(&barrier);
    let worker = std::thread::spawn(move || {
        ui_barrier.wait();
        ui.update(|current| current.with_enable_autocomplete(false))
            .unwrap();
    });
    barrier.wait();
    tray.update(|current| current.with_enable_autocorrections(false))
        .unwrap();
    worker.join().unwrap();
    let expected = AppSettings::default()
        .with_enable_autocomplete(false)
        .with_enable_autocorrections(false);
    c.check_fact_expect(
        "both live changes preserved",
        fixture.store.load().unwrap(),
        expected,
    );
    c.check_fact_expect(
        "both canonical changes preserved",
        Database::open(&fixture.path).unwrap().settings().unwrap(),
        expected,
    );
    c.assert_certify();
}

#[test]
fn hot_path_reads_do_not_wait_for_the_database_writer() {
    let mut c = certification("keyboard settings reads do not acquire SQLite writer");
    let fixture = Fixture::new();
    let writer_guard = fixture.store.writer.lock().unwrap();
    let reader = fixture.store.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let application = ApplicationIdentity::try_new(r"C:\Apps\example.exe").unwrap();
        sender
            .send((
                reader.load().unwrap(),
                reader.correction_policy(Some(&application)),
            ))
            .unwrap();
    });
    let result = receiver.recv_timeout(std::time::Duration::from_secs(2));
    drop(writer_guard);
    worker.join().unwrap();
    c.check_fact_expect(
        "read succeeds while writer is held",
        result,
        Ok((AppSettings::default(), CorrectionFeaturePolicy::ALL)),
    );
    c.assert_certify();
}

#[test]
fn unavailable_snapshot_disables_automatic_edits() {
    let mut c = certification("unavailable settings fail closed");
    let store = SettingsStore::new(AppSettings::default());
    let current = Arc::clone(&store.current);
    let worker = std::thread::spawn(move || {
        let _guard = current.write().unwrap();
        panic!("poison snapshot");
    });
    let _ = worker.join();
    let application = ApplicationIdentity::try_new(r"C:\Apps\example.exe").unwrap();
    c.check_fact_expect(
        "automatic classes denied",
        store.correction_policy(Some(&application)),
        CorrectionFeaturePolicy::new(false, false),
    );
    c.check_fact_expect(
        "global load reports unavailable",
        store.load(),
        Err(SettingsStoreUnavailable),
    );
    c.assert_certify();
}
