use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::{Arc, Mutex, RwLock};

use crate::application_policy::{ApplicationIdentity, ApplicationOverrides};
use crate::correction::CorrectionFeaturePolicy;
use crate::persistence::{AppSettings, Database, DatabaseError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsStoreUnavailable;

impl Display for SettingsStoreUnavailable {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("settings snapshot is unavailable")
    }
}

impl Error for SettingsStoreUnavailable {}

#[derive(Debug)]
pub enum SettingsWriteError {
    Database(DatabaseError),
    Unavailable,
}

impl Display for SettingsWriteError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(error) => Display::fmt(error, f),
            Self::Unavailable => f.write_str("settings writer is unavailable"),
        }
    }
}

impl Error for SettingsWriteError {}

impl From<DatabaseError> for SettingsWriteError {
    fn from(error: DatabaseError) -> Self {
        Self::Database(error)
    }
}

#[derive(Debug)]
struct SettingsSnapshot {
    global: AppSettings,
    applications: Arc<ApplicationOverrides>,
}

/// Rebuildable RAM projection plus one serialized persistence-first mutation gateway.
/// The writer mutex is never acquired by keyboard readers; SQLite writes do not
/// hold the snapshot lock, including during busy-timeout waits.
#[derive(Clone)]
pub struct SettingsStore {
    current: Arc<RwLock<SettingsSnapshot>>,
    writer: Arc<Mutex<Option<Database>>>,
}

impl std::fmt::Debug for SettingsStore {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsStore")
            .field("current", &self.current)
            .finish_non_exhaustive()
    }
}

impl SettingsStore {
    /// Detached read-only projection for probes/tests without persistence.
    pub fn new(settings: AppSettings) -> Self {
        Self {
            current: Arc::new(RwLock::new(SettingsSnapshot {
                global: settings,
                applications: Arc::default(),
            })),
            writer: Arc::new(Mutex::new(None)),
        }
    }

    pub fn from_database(database: Database) -> Result<Self, DatabaseError> {
        let global = database.settings()?;
        let applications = Arc::new(database.load_application_overrides()?);
        Ok(Self {
            current: Arc::new(RwLock::new(SettingsSnapshot {
                global,
                applications,
            })),
            writer: Arc::new(Mutex::new(Some(database))),
        })
    }

    pub fn load(&self) -> Result<AppSettings, SettingsStoreUnavailable> {
        self.current
            .read()
            .map(|snapshot| snapshot.global)
            .map_err(|_| SettingsStoreUnavailable)
    }

    pub fn application_overrides(
        &self,
    ) -> Result<Arc<ApplicationOverrides>, SettingsStoreUnavailable> {
        self.current
            .read()
            .map(|snapshot| Arc::clone(&snapshot.applications))
            .map_err(|_| SettingsStoreUnavailable)
    }

    pub fn correction_policy(
        &self,
        foreground: Option<&ApplicationIdentity>,
    ) -> CorrectionFeaturePolicy {
        self.current
            .read()
            .map(|snapshot| {
                snapshot
                    .applications
                    .effective_policy(snapshot.global, foreground)
            })
            .unwrap_or_else(|_| CorrectionFeaturePolicy::new(false, false))
    }

    pub fn input_suppressed(&self, foreground: Option<&ApplicationIdentity>) -> bool {
        self.current
            .read()
            .map(|snapshot| snapshot.applications.suppresses_input(foreground))
            .unwrap_or(true)
    }

    /// Derive the change from the latest canonical projection under the shared
    /// writer gate, not from a possibly stale UI copy of the whole settings row.
    pub fn update(
        &self,
        change: impl FnOnce(AppSettings) -> AppSettings,
    ) -> Result<AppSettings, SettingsWriteError> {
        let writer = self
            .writer
            .lock()
            .map_err(|_| SettingsWriteError::Unavailable)?;
        let database = writer.as_ref().ok_or(SettingsWriteError::Unavailable)?;
        let next = change(self.load().map_err(|_| SettingsWriteError::Unavailable)?);
        database.save_settings(next)?;
        self.current
            .write()
            .map_err(|_| SettingsWriteError::Unavailable)?
            .global = next;
        Ok(next)
    }

    pub fn set_application_suppressed(
        &self,
        application: &ApplicationIdentity,
        suppressed: bool,
    ) -> Result<(), SettingsWriteError> {
        let writer = self
            .writer
            .lock()
            .map_err(|_| SettingsWriteError::Unavailable)?;
        let database = writer.as_ref().ok_or(SettingsWriteError::Unavailable)?;
        let current = self
            .application_overrides()
            .map_err(|_| SettingsWriteError::Unavailable)?;
        let mut applications: Vec<_> = current
            .applications()
            .filter(|entry| *entry != application)
            .cloned()
            .collect();
        if suppressed {
            applications.push(application.clone());
        }
        let next = Arc::new(ApplicationOverrides::from_applications(applications));
        database.set_application_suppressed(application, suppressed)?;
        self.current
            .write()
            .map_err(|_| SettingsWriteError::Unavailable)?
            .applications = next;
        Ok(())
    }
}

#[cfg(test)]
#[path = "settings_certification.rs"]
mod certification;
