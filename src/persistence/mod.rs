mod database;

pub use database::{
    AppSettings, ClipboardEntryContent, ClipboardEntryId, ClipboardEntryKind, ClipboardEntryList,
    ClipboardEntryView, ClipboardHistoryLimit, ClipboardReorderPosition, CorrectionEventId,
    CorrectionUndoPlan, Database, DatabaseError, SettingsError, UndoHotkey,
};

#[cfg(test)]
mod database_certification;
#[cfg(test)]
mod database_tests;
