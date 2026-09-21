mod database;

pub use database::{
    AppSettings, ClipboardHistoryLimit, CorrectionEventId, CorrectionUndoPlan, Database,
    DatabaseError, SettingsError, UndoHotkey,
};

#[cfg(test)]
mod database_certification;
#[cfg(test)]
mod database_tests;
