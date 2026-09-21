use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;

use rusqlite::{Connection, TransactionBehavior, params};

use crate::language::{
    DictionaryEntry, LanguageId, LanguagePack, LanguagePackError, language_pack_from_entries,
    normalize_word,
};
use crate::lexicon::{UserLexicon, UserLexiconError, UserWord};

const CURRENT_SCHEMA_VERSION: i64 = 1;
const SCHEMA_V1: &str = r#"
CREATE TABLE app_settings (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    clipboard_history_limit INTEGER NOT NULL CHECK (clipboard_history_limit > 0),
    undo_hotkey TEXT NOT NULL DEFAULT 'pause'
) STRICT;

INSERT INTO app_settings (singleton, clipboard_history_limit, undo_hotkey)
VALUES (1, 1000, 'pause');

CREATE TABLE languages (
    id INTEGER PRIMARY KEY,
    code TEXT NOT NULL COLLATE NOCASE UNIQUE CHECK (length(trim(code)) > 0),
    display_name TEXT NOT NULL CHECK (length(trim(display_name)) > 0),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1))
) STRICT;

INSERT INTO languages (code, display_name) VALUES
    ('ru', 'Russian'),
    ('en', 'English');

CREATE TABLE dictionary_words (
    id INTEGER PRIMARY KEY,
    language_id INTEGER NOT NULL REFERENCES languages(id) ON DELETE CASCADE,
    term TEXT NOT NULL CHECK (length(term) > 0),
    normalized_term TEXT NOT NULL CHECK (length(normalized_term) > 0),
    frequency INTEGER NOT NULL CHECK (frequency > 0),
    UNIQUE (language_id, normalized_term)
) STRICT;

WITH seed(term, frequency) AS (VALUES
    ('для', 1000),
    ('что', 980),
    ('это', 970),
    ('как', 960),
    ('привет', 940),
    ('жизнь', 935),
    ('хорошо', 930),
    ('ёлка', 925),
    ('можно', 920),
    ('объект', 915),
    ('нужно', 910),
    ('быстро', 905),
    ('если', 900),
    ('люди', 895),
    ('да', 890),
    ('эхо', 885),
    ('нет', 880),
    ('юг', 875),
    ('мир', 860),
    ('текст', 850),
    ('слово', 840),
    ('тест', 830),
    ('язык', 820),
    ('работа', 810),
    ('окно', 800),
    ('программа', 790),
    ('исправление', 780),
    ('ошибка', 770),
    ('ошибки', 760),
    ('русский', 750),
    ('английский', 740),
    ('сейчас', 730),
    ('потом', 720),
    ('пример', 710),
    ('клавиатура', 700),
    ('предложение', 690)
)
INSERT INTO dictionary_words (language_id, term, normalized_term, frequency)
SELECT languages.id, seed.term, seed.term, seed.frequency
FROM seed CROSS JOIN languages
WHERE languages.code = 'ru';

WITH seed(term, frequency) AS (VALUES
    ('the', 1000),
    ('and', 990),
    ('this', 970),
    ('that', 960),
    ('hello', 950),
    ('world', 940),
    ('for', 930),
    ('with', 920),
    ('yes', 900),
    ('no', 890),
    ('if', 880),
    ('can', 870),
    ('need', 860),
    ('text', 850),
    ('word', 840),
    ('test', 830),
    ('language', 820),
    ('work', 810),
    ('window', 800),
    ('program', 790),
    ('correction', 780),
    ('error', 770),
    ('errors', 760),
    ('english', 750),
    ('russian', 740),
    ('now', 730),
    ('later', 720),
    ('example', 710),
    ('keyboard', 700),
    ('sentence', 690)
)
INSERT INTO dictionary_words (language_id, term, normalized_term, frequency)
SELECT languages.id, seed.term, seed.term, seed.frequency
FROM seed CROSS JOIN languages
WHERE languages.code = 'en';

WITH surface(term, frequency) AS (VALUES
    ('этот', 900), ('эта', 895), ('эту', 890), ('этой', 885), ('эти', 880),
    ('этого', 875), ('этому', 870), ('этим', 865), ('этом', 860), ('этих', 855), ('этими', 850),
    ('привета', 820), ('привету', 810), ('приветом', 800), ('привете', 790),
    ('приветы', 780), ('приветов', 770), ('приветам', 760), ('приветами', 750), ('приветах', 740),
    ('жизни', 900), ('жизнью', 850), ('жизней', 840), ('жизням', 800), ('жизнями', 790), ('жизнях', 780),
    ('ёлки', 880), ('ёлке', 850), ('ёлку', 850), ('ёлкой', 820), ('ёлок', 810), ('ёлкам', 780), ('ёлками', 770), ('ёлках', 760),
    ('объекта', 890), ('объекту', 860), ('объектом', 850), ('объекте', 840), ('объекты', 850),
    ('объектов', 840), ('объектам', 800), ('объектами', 790), ('объектах', 780),
    ('людей', 890), ('людям', 850), ('людьми', 840), ('людях', 820),
    ('мира', 840), ('миру', 820), ('миром', 810), ('мире', 800), ('миры', 780),
    ('миров', 790), ('мирам', 760), ('мирами', 750), ('мирах', 740),
    ('текста', 830), ('тексту', 810), ('текстом', 800), ('тексте', 790), ('тексты', 800),
    ('текстов', 790), ('текстам', 760), ('текстами', 750), ('текстах', 740),
    ('слова', 835), ('слову', 830), ('словом', 820), ('слове', 810), ('слов', 800),
    ('словам', 780), ('словами', 770), ('словах', 760),
    ('теста', 810), ('тесту', 790), ('тестом', 780), ('тесте', 770), ('тесты', 790),
    ('тестов', 780), ('тестам', 750), ('тестами', 740), ('тестах', 730),
    ('языка', 810), ('языку', 790), ('языком', 780), ('языке', 770), ('языки', 790),
    ('языков', 780), ('языкам', 750), ('языками', 740), ('языках', 730),
    ('работы', 800), ('работе', 790), ('работу', 790), ('работой', 770), ('работ', 760),
    ('работам', 740), ('работами', 730), ('работах', 720),
    ('окна', 800), ('окну', 780), ('окном', 770), ('окне', 760), ('окон', 750),
    ('окнам', 730), ('окнами', 720), ('окнах', 710),
    ('программы', 790), ('программе', 780), ('программу', 780), ('программой', 760), ('программ', 750),
    ('программам', 730), ('программами', 720), ('программах', 710),
    ('исправления', 780), ('исправлению', 760), ('исправлением', 750), ('исправлении', 740),
    ('исправлений', 730), ('исправлениям', 710), ('исправлениями', 700), ('исправлениях', 690),
    ('ошибке', 750), ('ошибку', 750), ('ошибкой', 730), ('ошибок', 720),
    ('ошибкам', 700), ('ошибками', 690), ('ошибках', 680),
    ('русского', 740), ('русскому', 730), ('русским', 720), ('русском', 710),
    ('русская', 730), ('русской', 720), ('русскую', 710), ('русское', 720),
    ('русские', 720), ('русских', 710), ('русскими', 700),
    ('английского', 730), ('английскому', 720), ('английским', 710), ('английском', 700),
    ('английская', 720), ('английской', 710), ('английскую', 700), ('английское', 710),
    ('английские', 710), ('английских', 700), ('английскими', 690),
    ('примера', 700), ('примеру', 690), ('примером', 680), ('примере', 670), ('примеры', 690),
    ('примеров', 680), ('примерам', 660), ('примерами', 650), ('примерах', 640),
    ('клавиатуры', 690), ('клавиатуре', 680), ('клавиатуру', 680), ('клавиатурой', 660),
    ('клавиатур', 650), ('клавиатурам', 630), ('клавиатурами', 620), ('клавиатурах', 610),
    ('предложения', 680), ('предложению', 660), ('предложением', 650), ('предложении', 640),
    ('предложений', 630), ('предложениям', 610), ('предложениями', 600), ('предложениях', 590)
)
INSERT OR IGNORE INTO dictionary_words (language_id, term, normalized_term, frequency)
SELECT languages.id, surface.term, surface.term, surface.frequency
FROM surface CROSS JOIN languages
WHERE languages.code = 'ru';

WITH surface(term, frequency) AS (VALUES
    ('these', 940), ('those', 930),
    ('worlds', 900), ('needs', 850), ('needed', 840), ('needing', 810),
    ('texts', 820), ('words', 820), ('tests', 810), ('languages', 800),
    ('works', 790), ('worked', 780), ('working', 800), ('windows', 790),
    ('programs', 780), ('programmed', 750), ('programming', 770),
    ('corrections', 760), ('examples', 700), ('keyboards', 690), ('sentences', 680), ('hellos', 650)
)
INSERT OR IGNORE INTO dictionary_words (language_id, term, normalized_term, frequency)
SELECT languages.id, surface.term, surface.term, surface.frequency
FROM surface CROSS JOIN languages
WHERE languages.code = 'en';

CREATE TABLE user_words (
    id INTEGER PRIMARY KEY,
    term TEXT NOT NULL CHECK (length(term) > 0),
    normalized_term TEXT NOT NULL UNIQUE CHECK (length(normalized_term) > 0),
    use_count INTEGER NOT NULL DEFAULT 1 CHECK (use_count > 0),
    last_used_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE correction_events (
    id INTEGER PRIMARY KEY,
    observed_text TEXT NOT NULL CHECK (length(observed_text) > 0),
    replacement_text TEXT NOT NULL CHECK (length(replacement_text) > 0),
    created_at_ms INTEGER NOT NULL,
    undone_at_ms INTEGER
) STRICT;

CREATE TABLE text_history (
    id INTEGER PRIMARY KEY,
    text TEXT NOT NULL UNIQUE CHECK (length(text) > 0),
    last_used_at_ms INTEGER NOT NULL,
    use_count INTEGER NOT NULL DEFAULT 1 CHECK (use_count > 0)
) STRICT;

CREATE TABLE clipboard_entries (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('text', 'image', 'files')),
    last_seen_at_ms INTEGER NOT NULL,
    content_hash BLOB NOT NULL,
    copy_count INTEGER NOT NULL DEFAULT 1 CHECK (copy_count > 0),
    pinned_at_ms INTEGER,
    UNIQUE (kind, content_hash)
) STRICT;

CREATE INDEX clipboard_entries_recent_idx
    ON clipboard_entries(last_seen_at_ms DESC);

CREATE INDEX clipboard_entries_pinned_idx
    ON clipboard_entries(pinned_at_ms DESC)
    WHERE pinned_at_ms IS NOT NULL;

CREATE TABLE clipboard_text (
    entry_id INTEGER PRIMARY KEY REFERENCES clipboard_entries(id) ON DELETE CASCADE,
    text TEXT NOT NULL
) STRICT;

CREATE TABLE clipboard_images (
    entry_id INTEGER PRIMARY KEY REFERENCES clipboard_entries(id) ON DELETE CASCADE,
    format TEXT NOT NULL CHECK (length(format) > 0),
    data BLOB NOT NULL CHECK (length(data) > 0)
) STRICT;

CREATE TABLE clipboard_files (
    entry_id INTEGER NOT NULL REFERENCES clipboard_entries(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    path TEXT NOT NULL CHECK (length(path) > 0),
    PRIMARY KEY (entry_id, position)
) STRICT;
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardHistoryLimit(u32);

impl ClipboardHistoryLimit {
    pub const DEFAULT: Self = Self(1000);

    pub fn try_new(value: u32) -> Result<Self, SettingsError> {
        if value == 0 {
            return Err(SettingsError::ClipboardHistoryLimitMustBePositive);
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    fn try_from_stored(value: i64) -> Result<Self, DatabaseError> {
        let value = u32::try_from(value)
            .map_err(|_| DatabaseError::InvalidStoredClipboardHistoryLimit(value))?;
        Self::try_new(value)
            .map_err(|_| DatabaseError::InvalidStoredClipboardHistoryLimit(value as i64))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoHotkey {
    Pause,
}

impl UndoHotkey {
    pub const DEFAULT: Self = Self::Pause;

    pub const fn as_stored(self) -> &'static str {
        match self {
            Self::Pause => "pause",
        }
    }

    fn try_from_stored(value: &str) -> Result<Self, DatabaseError> {
        match value {
            "pause" => Ok(Self::Pause),
            _ => Err(DatabaseError::InvalidStoredUndoHotkey(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppSettings {
    clipboard_history_limit: ClipboardHistoryLimit,
    undo_hotkey: UndoHotkey,
}

impl AppSettings {
    pub const fn new(
        clipboard_history_limit: ClipboardHistoryLimit,
        undo_hotkey: UndoHotkey,
    ) -> Self {
        Self {
            clipboard_history_limit,
            undo_hotkey,
        }
    }

    pub const fn clipboard_history_limit(self) -> ClipboardHistoryLimit {
        self.clipboard_history_limit
    }

    pub const fn undo_hotkey(self) -> UndoHotkey {
        self.undo_hotkey
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self::new(ClipboardHistoryLimit::DEFAULT, UndoHotkey::DEFAULT)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsError {
    ClipboardHistoryLimitMustBePositive,
}

impl Display for SettingsError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClipboardHistoryLimitMustBePositive => {
                formatter.write_str("clipboard history limit must be positive")
            }
        }
    }
}

impl Error for SettingsError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorrectionEventId(i64);

impl CorrectionEventId {
    pub const fn get(self) -> i64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrectionUndoPlan {
    event_id: CorrectionEventId,
    original_text: String,
    replacement_text: String,
}

impl CorrectionUndoPlan {
    pub const fn event_id(&self) -> CorrectionEventId {
        self.event_id
    }

    pub fn original_text(&self) -> &str {
        &self.original_text
    }

    pub fn replacement_text(&self) -> &str {
        &self.replacement_text
    }
}

#[derive(Debug)]
pub enum DatabaseError {
    Sqlite(rusqlite::Error),
    LanguagePack(LanguagePackError),
    UserLexicon(UserLexiconError),
    SchemaTooNew {
        found: i64,
        supported: i64,
    },
    InvalidStoredClipboardHistoryLimit(i64),
    InvalidStoredUndoHotkey(String),
    InvalidStoredDictionaryFrequency(i64),
    InvalidStoredUserWordUseCount(i64),
    InvalidCorrectionText,
    CorrectionEventNotFound(i64),
    CorrectionEventAlreadyUndone(i64),
    InvalidStoredNormalizedTerm {
        term: String,
        normalized_term: String,
    },
    MissingAppSettings,
}

impl Display for DatabaseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "SQLite error: {error}"),
            Self::LanguagePack(error) => write!(formatter, "invalid language data: {error:?}"),
            Self::UserLexicon(error) => write!(formatter, "invalid user lexicon data: {error}"),
            Self::SchemaTooNew { found, supported } => write!(
                formatter,
                "database schema version {found} is newer than supported version {supported}"
            ),
            Self::InvalidStoredClipboardHistoryLimit(value) => {
                write!(formatter, "invalid stored clipboard history limit: {value}")
            }
            Self::InvalidStoredUndoHotkey(value) => {
                write!(formatter, "invalid stored undo hotkey: {value:?}")
            }
            Self::InvalidStoredDictionaryFrequency(value) => {
                write!(formatter, "invalid stored dictionary frequency: {value}")
            }
            Self::InvalidStoredUserWordUseCount(value) => {
                write!(formatter, "invalid stored user word use count: {value}")
            }
            Self::InvalidCorrectionText => formatter.write_str("invalid correction event text"),
            Self::CorrectionEventNotFound(id) => {
                write!(formatter, "correction event {id} was not found")
            }
            Self::CorrectionEventAlreadyUndone(id) => {
                write!(formatter, "correction event {id} was already undone")
            }
            Self::InvalidStoredNormalizedTerm {
                term,
                normalized_term,
            } => write!(
                formatter,
                "stored normalized dictionary term {normalized_term:?} does not match {term:?}"
            ),
            Self::MissingAppSettings => formatter.write_str("app settings row is missing"),
        }
    }
}

impl Error for DatabaseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::UserLexicon(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for DatabaseError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<LanguagePackError> for DatabaseError {
    fn from(error: LanguagePackError) -> Self {
        Self::LanguagePack(error)
    }
}

impl From<UserLexiconError> for DatabaseError {
    fn from(error: UserLexiconError) -> Self {
        Self::UserLexicon(error)
    }
}

pub struct Database {
    connection: Connection,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DatabaseError> {
        let connection = Connection::open(path)?;
        Self::from_connection(connection)
    }

    pub fn open_in_memory() -> Result<Self, DatabaseError> {
        let connection = Connection::open_in_memory()?;
        Self::from_connection(connection)
    }

    fn from_connection(mut connection: Connection) -> Result<Self, DatabaseError> {
        connection.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&mut connection)?;
        Ok(Self { connection })
    }

    #[cfg(test)]
    pub(super) fn schema_version(&self) -> Result<i64, DatabaseError> {
        schema_version(&self.connection)
    }

    pub fn settings(&self) -> Result<AppSettings, DatabaseError> {
        let result = self.connection.query_row(
            "SELECT clipboard_history_limit, undo_hotkey FROM app_settings WHERE singleton = 1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        );
        let (stored_limit, stored_undo_hotkey) = match result {
            Ok(value) => value,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Err(DatabaseError::MissingAppSettings);
            }
            Err(error) => return Err(DatabaseError::Sqlite(error)),
        };
        Ok(AppSettings::new(
            ClipboardHistoryLimit::try_from_stored(stored_limit)?,
            UndoHotkey::try_from_stored(&stored_undo_hotkey)?,
        ))
    }

    pub fn save_settings(&self, settings: AppSettings) -> Result<(), DatabaseError> {
        let changed = self.connection.execute(
            "UPDATE app_settings SET clipboard_history_limit = ?1, undo_hotkey = ?2 WHERE singleton = 1",
            params![
                i64::from(settings.clipboard_history_limit().get()),
                settings.undo_hotkey().as_stored(),
            ],
        )?;
        if changed != 1 {
            return Err(DatabaseError::MissingAppSettings);
        }
        Ok(())
    }

    pub fn load_enabled_language_packs(&self) -> Result<Vec<LanguagePack>, DatabaseError> {
        let mut language_statement = self
            .connection
            .prepare("SELECT id, code FROM languages WHERE enabled = 1 ORDER BY id")?;
        let languages: Vec<(i64, String)> = language_statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        drop(language_statement);

        let mut packs = Vec::with_capacity(languages.len());
        for (database_id, code) in languages {
            let mut word_statement = self.connection.prepare(
                "SELECT term, normalized_term, frequency FROM dictionary_words WHERE language_id = ?1 ORDER BY id",
            )?;
            let stored_entries: Vec<(String, String, i64)> = word_statement
                .query_map([database_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?
                .collect::<Result<_, _>>()?;

            let mut entries = Vec::with_capacity(stored_entries.len());
            for (term, normalized_term, stored_frequency) in stored_entries {
                let expected_normalized = normalize_word(&term);
                if normalized_term != expected_normalized {
                    return Err(DatabaseError::InvalidStoredNormalizedTerm {
                        term,
                        normalized_term,
                    });
                }
                let frequency = u32::try_from(stored_frequency).map_err(|_| {
                    DatabaseError::InvalidStoredDictionaryFrequency(stored_frequency)
                })?;
                entries.push(DictionaryEntry::try_new(term, frequency)?);
            }

            let id = LanguageId::try_new(code)?;
            packs.push(language_pack_from_entries(id, entries)?);
        }
        Ok(packs)
    }

    pub fn record_user_word(&self, term: &str, used_at_ms: i64) -> Result<UserWord, DatabaseError> {
        let input = UserWord::try_new(term, 1, used_at_ms)?;
        upsert_user_word(&self.connection, &input)
    }

    pub fn record_correction_event(
        &self,
        observed_text: &str,
        replacement_text: &str,
        created_at_ms: i64,
    ) -> Result<CorrectionEventId, DatabaseError> {
        validate_correction_text(observed_text)?;
        validate_correction_text(replacement_text)?;
        self.connection.execute(
            "INSERT INTO correction_events (observed_text, replacement_text, created_at_ms) VALUES (?1, ?2, ?3)",
            params![observed_text, replacement_text, created_at_ms],
        )?;
        Ok(CorrectionEventId(self.connection.last_insert_rowid()))
    }

    pub fn prepare_correction_undo(
        &self,
        event_id: CorrectionEventId,
    ) -> Result<CorrectionUndoPlan, DatabaseError> {
        let result = self.connection.query_row(
            "SELECT observed_text, replacement_text, undone_at_ms FROM correction_events WHERE id = ?1",
            [event_id.get()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            },
        );
        let (original_text, replacement_text, undone_at_ms) = match result {
            Ok(row) => row,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Err(DatabaseError::CorrectionEventNotFound(event_id.get()));
            }
            Err(error) => return Err(DatabaseError::Sqlite(error)),
        };
        if undone_at_ms.is_some() {
            return Err(DatabaseError::CorrectionEventAlreadyUndone(event_id.get()));
        }
        Ok(CorrectionUndoPlan {
            event_id,
            original_text,
            replacement_text,
        })
    }

    pub fn commit_correction_undo(
        &mut self,
        event_id: CorrectionEventId,
        undone_at_ms: i64,
    ) -> Result<UserWord, DatabaseError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = transaction.query_row(
            "SELECT observed_text, undone_at_ms FROM correction_events WHERE id = ?1",
            [event_id.get()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?)),
        );
        let (original_text, previous_undo) = match result {
            Ok(row) => row,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Err(DatabaseError::CorrectionEventNotFound(event_id.get()));
            }
            Err(error) => return Err(DatabaseError::Sqlite(error)),
        };
        if previous_undo.is_some() {
            return Err(DatabaseError::CorrectionEventAlreadyUndone(event_id.get()));
        }

        let accepted = UserWord::try_new(original_text, 1, undone_at_ms)?;
        let stored = upsert_user_word(&transaction, &accepted)?;
        let changed = transaction.execute(
            "UPDATE correction_events SET undone_at_ms = ?1 WHERE id = ?2 AND undone_at_ms IS NULL",
            params![undone_at_ms, event_id.get()],
        )?;
        if changed != 1 {
            return Err(DatabaseError::CorrectionEventAlreadyUndone(event_id.get()));
        }
        transaction.commit()?;
        Ok(stored)
    }

    pub fn load_user_lexicon(&self) -> Result<UserLexicon, DatabaseError> {
        let mut statement = self.connection.prepare(
            "SELECT term, normalized_term, use_count, last_used_at_ms FROM user_words ORDER BY normalized_term",
        )?;
        let stored: Vec<(String, String, i64, i64)> = statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<Result<_, _>>()?;
        let mut words = Vec::with_capacity(stored.len());
        for row in stored {
            words.push(user_word_from_stored(row)?);
        }
        Ok(UserLexicon::try_new(words)?)
    }
}

fn validate_correction_text(text: &str) -> Result<(), DatabaseError> {
    if text.is_empty() || text.contains('\0') {
        return Err(DatabaseError::InvalidCorrectionText);
    }
    Ok(())
}

fn upsert_user_word(connection: &Connection, input: &UserWord) -> Result<UserWord, DatabaseError> {
    let stored = connection.query_row(
        r#"
INSERT INTO user_words (term, normalized_term, use_count, last_used_at_ms)
VALUES (?1, ?2, 1, ?3)
ON CONFLICT(normalized_term) DO UPDATE SET
    use_count = user_words.use_count + 1,
    last_used_at_ms = MAX(user_words.last_used_at_ms, excluded.last_used_at_ms)
RETURNING term, normalized_term, use_count, last_used_at_ms
"#,
        params![
            input.term(),
            input.normalized_term(),
            input.last_used_at_ms()
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        },
    )?;
    user_word_from_stored(stored)
}

fn user_word_from_stored(stored: (String, String, i64, i64)) -> Result<UserWord, DatabaseError> {
    let (term, normalized_term, stored_use_count, last_used_at_ms) = stored;
    if normalize_word(&term) != normalized_term {
        return Err(DatabaseError::InvalidStoredNormalizedTerm {
            term,
            normalized_term,
        });
    }
    let use_count = u32::try_from(stored_use_count)
        .map_err(|_| DatabaseError::InvalidStoredUserWordUseCount(stored_use_count))?;
    Ok(UserWord::try_new(term, use_count, last_used_at_ms)?)
}

fn schema_version(connection: &Connection) -> Result<i64, DatabaseError> {
    Ok(connection.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

fn migrate(connection: &mut Connection) -> Result<(), DatabaseError> {
    let version = schema_version(connection)?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(DatabaseError::SchemaTooNew {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }

    if version < CURRENT_SCHEMA_VERSION {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(SCHEMA_V1)?;
        transaction.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)?;
        transaction.commit()?;
    }

    Ok(())
}
