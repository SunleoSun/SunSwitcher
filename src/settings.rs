use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::{Arc, RwLock};

use crate::persistence::AppSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsStoreUnavailable;

impl Display for SettingsStoreUnavailable {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("settings snapshot is unavailable")
    }
}

impl Error for SettingsStoreUnavailable {}

#[derive(Debug, Clone)]
pub struct SettingsStore {
    current: Arc<RwLock<AppSettings>>,
}

impl SettingsStore {
    pub fn new(settings: AppSettings) -> Self {
        Self {
            current: Arc::new(RwLock::new(settings)),
        }
    }

    pub fn load(&self) -> Result<AppSettings, SettingsStoreUnavailable> {
        self.current
            .read()
            .map(|settings| *settings)
            .map_err(|_| SettingsStoreUnavailable)
    }

    pub fn replace(&self, settings: AppSettings) -> Result<(), SettingsStoreUnavailable> {
        let mut slot = self.current.write().map_err(|_| SettingsStoreUnavailable)?;
        *slot = settings;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_store_replaces_the_live_snapshot() {
        let store = SettingsStore::new(AppSettings::default());
        let next = store.load().unwrap().with_enable_autocomplete(false);
        store.replace(next).unwrap();
        assert!(!store.load().unwrap().enable_autocomplete());
    }
}
