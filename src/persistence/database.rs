use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, TransactionBehavior, params};

use crate::completion::sequence::{SequenceCandidate, SequenceHistory, SequenceHistoryError};
use crate::language::{
    LanguageId, LanguagePack, LanguagePackError, builtin_language_pack, normalize_word,
};
use crate::lexicon::{IgnoredWords, UserLexicon, UserLexiconError, UserWord};

const CURRENT_SCHEMA_VERSION: i64 = 8;
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

const SCHEMA_V2: &str = r#"
CREATE TABLE completion_hidden_words (
    normalized_term TEXT PRIMARY KEY CHECK (length(normalized_term) > 0)
) STRICT;
"#;

// Grave/backtick is a layout-ambiguous physical key (` in EN, ё in RU), not part of the
// canonical lexical token grammar. Earlier typed learning could persist an edge grave as if it
// were part of a word after the lexical provider had already validated the punctuation-free core.
const SCHEMA_V3: &str = r#"
DELETE FROM user_words WHERE instr(term, char(96)) > 0;
DELETE FROM text_history WHERE instr(text, char(96)) > 0;
"#;

// Built-in RU/EN vocabulary moved to compact immutable FST assets. Version 5 owns the
// final cleanup by dropping the obsolete mutable dictionary table entirely.
const SCHEMA_V4: &str = r#""#;

const SCHEMA_V5: &str = r#"
DROP TABLE IF EXISTS dictionary_words;
"#;

const SCHEMA_V6: &str = r#"
UPDATE clipboard_entries SET current_order_index = last_seen_at_ms WHERE current_order_index = 0;
UPDATE clipboard_entries SET pinned_order_index = pinned_at_ms WHERE pinned_order_index IS NULL AND pinned_at_ms IS NOT NULL;
DROP INDEX IF EXISTS clipboard_entries_recent_idx;
DROP INDEX IF EXISTS clipboard_entries_pinned_idx;
CREATE INDEX IF NOT EXISTS clipboard_entries_current_order_idx
ON clipboard_entries(current_order_index DESC, id DESC);
CREATE INDEX IF NOT EXISTS clipboard_entries_pinned_order_idx
ON clipboard_entries(pinned_order_index DESC, id DESC)
WHERE pinned_at_ms IS NOT NULL;
"#;

const SCHEMA_V7: &str = r#"
CREATE TABLE IF NOT EXISTS ignored_words (
    normalized_term TEXT PRIMARY KEY CHECK (length(normalized_term) > 0)
) STRICT;
"#;

const APP_SETTINGS_V8_COLUMNS: [(&str, &str); 9] = [
    (
        "enable_autocomplete",
        "ALTER TABLE app_settings ADD COLUMN enable_autocomplete INTEGER NOT NULL DEFAULT 1 CHECK (enable_autocomplete IN (0, 1))",
    ),
    (
        "enable_autocorrections",
        "ALTER TABLE app_settings ADD COLUMN enable_autocorrections INTEGER NOT NULL DEFAULT 1 CHECK (enable_autocorrections IN (0, 1))",
    ),
    (
        "enable_auto_keyboard_switches",
        "ALTER TABLE app_settings ADD COLUMN enable_auto_keyboard_switches INTEGER NOT NULL DEFAULT 1 CHECK (enable_auto_keyboard_switches IN (0, 1))",
    ),
    (
        "notification_timeout_seconds",
        "ALTER TABLE app_settings ADD COLUMN notification_timeout_seconds INTEGER NOT NULL DEFAULT 3 CHECK (notification_timeout_seconds BETWEEN 1 AND 60)",
    ),
    (
        "clipboard_current_hotkey",
        "ALTER TABLE app_settings ADD COLUMN clipboard_current_hotkey TEXT NOT NULL DEFAULT 'ctrl+shift+vk:109'",
    ),
    (
        "clipboard_pinned_hotkey",
        "ALTER TABLE app_settings ADD COLUMN clipboard_pinned_hotkey TEXT NOT NULL DEFAULT 'ctrl+shift+vk:107'",
    ),
    (
        "completion_accept_hotkey",
        "ALTER TABLE app_settings ADD COLUMN completion_accept_hotkey TEXT NOT NULL DEFAULT 'vk:9'",
    ),
    (
        "completion_next_word_hotkey",
        "ALTER TABLE app_settings ADD COLUMN completion_next_word_hotkey TEXT NOT NULL DEFAULT 'alt+vk:39'",
    ),
    (
        "ignore_word_hotkey",
        "ALTER TABLE app_settings ADD COLUMN ignore_word_hotkey TEXT NOT NULL DEFAULT 'ctrl+vk:19'",
    ),
];

const HOTKEY_CTRL: u8 = 1;
const HOTKEY_SHIFT: u8 = 2;
const HOTKEY_ALT: u8 = 4;
const HOTKEY_WIN: u8 = 8;
const WINDOWS_VK_CANCEL: u16 = 0x03;
const WINDOWS_VK_PAUSE: u16 = 0x13;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HotkeyAction {
    ClipboardCurrent,
    ClipboardPinned,
    CompletionAccept,
    CompletionNextWord,
    UndoOrForgetWord,
    IgnoreWord,
}

impl HotkeyAction {
    pub const ALL: [Self; 6] = [
        Self::ClipboardCurrent,
        Self::ClipboardPinned,
        Self::CompletionAccept,
        Self::CompletionNextWord,
        Self::UndoOrForgetWord,
        Self::IgnoreWord,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyConflict {
    Configured(HotkeyAction),
    ReservedCompletionControl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HotkeyModifiers(u8);

impl HotkeyModifiers {
    pub const NONE: Self = Self(0);

    pub const fn new(control: bool, shift: bool, alt: bool, win: bool) -> Self {
        Self(
            (if control { HOTKEY_CTRL } else { 0 })
                | (if shift { HOTKEY_SHIFT } else { 0 })
                | (if alt { HOTKEY_ALT } else { 0 })
                | (if win { HOTKEY_WIN } else { 0 }),
        )
    }

    pub const fn control(self) -> bool {
        self.0 & HOTKEY_CTRL != 0
    }

    pub const fn shift(self) -> bool {
        self.0 & HOTKEY_SHIFT != 0
    }

    pub const fn alt(self) -> bool {
        self.0 & HOTKEY_ALT != 0
    }

    pub const fn win(self) -> bool {
        self.0 & HOTKEY_WIN != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HotkeyBinding {
    key_code: u16,
    modifiers: HotkeyModifiers,
}

impl HotkeyBinding {
    pub const CLIPBOARD_CURRENT_DEFAULT: Self =
        Self::from_parts(0x6D, HotkeyModifiers::new(true, true, false, false));
    pub const CLIPBOARD_PINNED_DEFAULT: Self =
        Self::from_parts(0x6B, HotkeyModifiers::new(true, true, false, false));
    pub const COMPLETION_ACCEPT_DEFAULT: Self = Self::from_parts(0x09, HotkeyModifiers::NONE);
    pub const COMPLETION_NEXT_WORD_DEFAULT: Self =
        Self::from_parts(0x27, HotkeyModifiers::new(false, false, true, false));
    pub const UNDO_DEFAULT: Self = Self::from_parts(0x13, HotkeyModifiers::NONE);
    pub const IGNORE_WORD_DEFAULT: Self =
        Self::from_parts(0x13, HotkeyModifiers::new(true, false, false, false));

    const fn from_parts(key_code: u16, modifiers: HotkeyModifiers) -> Self {
        Self {
            key_code,
            modifiers,
        }
    }

    pub fn try_new(
        key_code: u16,
        control: bool,
        shift: bool,
        alt: bool,
        win: bool,
    ) -> Result<Self, SettingsError> {
        if key_code == 0 || key_code > 0xFF {
            return Err(SettingsError::InvalidHotkeyKeyCode(key_code));
        }
        let modifiers = HotkeyModifiers::new(control, shift, alt, win);
        // Windows reports Ctrl+Pause/Break as VK_CANCEL on common keyboards. Keep one
        // canonical persisted/display identity so capture and runtime matching cannot diverge.
        let key_code = if key_code == WINDOWS_VK_CANCEL && modifiers.control() {
            WINDOWS_VK_PAUSE
        } else {
            key_code
        };
        Ok(Self::from_parts(key_code, modifiers))
    }

    pub const fn key_code(self) -> u16 {
        self.key_code
    }

    pub const fn modifiers(self) -> HotkeyModifiers {
        self.modifiers
    }

    pub fn alternate_key_code(self, action: HotkeyAction) -> Option<u16> {
        match action {
            HotkeyAction::ClipboardCurrent if self == Self::CLIPBOARD_CURRENT_DEFAULT => Some(0xBD),
            HotkeyAction::ClipboardPinned if self == Self::CLIPBOARD_PINNED_DEFAULT => Some(0xBB),
            _ => None,
        }
    }

    pub fn as_stored(self) -> String {
        let mut parts = Vec::with_capacity(5);
        if self.modifiers.control() {
            parts.push("ctrl".to_owned());
        }
        if self.modifiers.shift() {
            parts.push("shift".to_owned());
        }
        if self.modifiers.alt() {
            parts.push("alt".to_owned());
        }
        if self.modifiers.win() {
            parts.push("win".to_owned());
        }
        parts.push(format!("vk:{}", self.key_code));
        parts.join("+")
    }

    fn try_from_stored(value: &str) -> Result<Self, SettingsError> {
        let mut control = false;
        let mut shift = false;
        let mut alt = false;
        let mut win = false;
        let mut key_code = None;
        for part in value.split('+') {
            match part {
                "ctrl" if !control => control = true,
                "shift" if !shift => shift = true,
                "alt" if !alt => alt = true,
                "win" if !win => win = true,
                _ if part.starts_with("vk:") && key_code.is_none() => {
                    key_code = part[3..].parse::<u16>().ok();
                    if key_code.is_none() {
                        return Err(SettingsError::InvalidHotkeyEncoding);
                    }
                }
                _ => return Err(SettingsError::InvalidHotkeyEncoding),
            }
        }
        let key_code = key_code.ok_or(SettingsError::InvalidHotkeyEncoding)?;
        Self::try_new(key_code, control, shift, alt, win)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoHotkey {
    Pause,
    Binding(HotkeyBinding),
}

impl UndoHotkey {
    pub const DEFAULT: Self = Self::Pause;

    pub const fn binding(self) -> HotkeyBinding {
        match self {
            Self::Pause => HotkeyBinding::UNDO_DEFAULT,
            Self::Binding(binding) => binding,
        }
    }

    pub const fn from_binding(binding: HotkeyBinding) -> Self {
        if binding.key_code == HotkeyBinding::UNDO_DEFAULT.key_code
            && binding.modifiers.0 == HotkeyBinding::UNDO_DEFAULT.modifiers.0
        {
            Self::Pause
        } else {
            Self::Binding(binding)
        }
    }

    fn try_from_stored(value: &str) -> Result<Self, DatabaseError> {
        if value == "pause" {
            return Ok(Self::Pause);
        }
        HotkeyBinding::try_from_stored(value)
            .map(Self::from_binding)
            .map_err(|_| DatabaseError::InvalidStoredUndoHotkey(value.to_owned()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationTimeoutSeconds(u32);

impl NotificationTimeoutSeconds {
    pub const DEFAULT: Self = Self(3);
    pub const MIN: u32 = 1;
    pub const MAX: u32 = 60;

    pub fn try_new(value: u32) -> Result<Self, SettingsError> {
        if !(Self::MIN..=Self::MAX).contains(&value) {
            return Err(SettingsError::NotificationTimeoutOutOfRange(value));
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    pub const fn duration(self) -> Duration {
        Duration::from_secs(self.0 as u64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppSettings {
    clipboard_history_limit: ClipboardHistoryLimit,
    undo_hotkey: HotkeyBinding,
    ignore_word_hotkey: HotkeyBinding,
    clipboard_current_hotkey: HotkeyBinding,
    clipboard_pinned_hotkey: HotkeyBinding,
    completion_accept_hotkey: HotkeyBinding,
    completion_next_word_hotkey: HotkeyBinding,
    enable_autocomplete: bool,
    enable_autocorrections: bool,
    enable_auto_keyboard_switches: bool,
    notification_timeout_seconds: NotificationTimeoutSeconds,
}

impl AppSettings {
    pub const fn new(
        clipboard_history_limit: ClipboardHistoryLimit,
        undo_hotkey: UndoHotkey,
    ) -> Self {
        Self {
            clipboard_history_limit,
            undo_hotkey: undo_hotkey.binding(),
            ignore_word_hotkey: HotkeyBinding::IGNORE_WORD_DEFAULT,
            clipboard_current_hotkey: HotkeyBinding::CLIPBOARD_CURRENT_DEFAULT,
            clipboard_pinned_hotkey: HotkeyBinding::CLIPBOARD_PINNED_DEFAULT,
            completion_accept_hotkey: HotkeyBinding::COMPLETION_ACCEPT_DEFAULT,
            completion_next_word_hotkey: HotkeyBinding::COMPLETION_NEXT_WORD_DEFAULT,
            enable_autocomplete: true,
            enable_autocorrections: true,
            enable_auto_keyboard_switches: true,
            notification_timeout_seconds: NotificationTimeoutSeconds::DEFAULT,
        }
    }

    pub const fn clipboard_history_limit(self) -> ClipboardHistoryLimit {
        self.clipboard_history_limit
    }

    pub const fn undo_hotkey(self) -> UndoHotkey {
        UndoHotkey::from_binding(self.undo_hotkey)
    }

    pub const fn hotkey(self, action: HotkeyAction) -> HotkeyBinding {
        match action {
            HotkeyAction::ClipboardCurrent => self.clipboard_current_hotkey,
            HotkeyAction::ClipboardPinned => self.clipboard_pinned_hotkey,
            HotkeyAction::CompletionAccept => self.completion_accept_hotkey,
            HotkeyAction::CompletionNextWord => self.completion_next_word_hotkey,
            HotkeyAction::UndoOrForgetWord => self.undo_hotkey,
            HotkeyAction::IgnoreWord => self.ignore_word_hotkey,
        }
    }

    pub const fn with_hotkey(mut self, action: HotkeyAction, binding: HotkeyBinding) -> Self {
        match action {
            HotkeyAction::ClipboardCurrent => self.clipboard_current_hotkey = binding,
            HotkeyAction::ClipboardPinned => self.clipboard_pinned_hotkey = binding,
            HotkeyAction::CompletionAccept => self.completion_accept_hotkey = binding,
            HotkeyAction::CompletionNextWord => self.completion_next_word_hotkey = binding,
            HotkeyAction::UndoOrForgetWord => self.undo_hotkey = binding,
            HotkeyAction::IgnoreWord => self.ignore_word_hotkey = binding,
        }
        self
    }

    pub fn hotkey_conflict(
        self,
        action: HotkeyAction,
        binding: HotkeyBinding,
    ) -> Option<HotkeyConflict> {
        if binding.modifiers() == HotkeyModifiers::NONE
            && matches!(binding.key_code(), 0x1B | 0x26 | 0x28 | 0x2E)
        {
            return Some(HotkeyConflict::ReservedCompletionControl);
        }
        HotkeyAction::ALL
            .into_iter()
            .find(|candidate| {
                if *candidate == action {
                    return false;
                }
                let existing = self.hotkey(*candidate);
                if existing == binding || existing.modifiers() != binding.modifiers() {
                    return existing == binding;
                }
                existing.alternate_key_code(*candidate) == Some(binding.key_code())
                    || binding.alternate_key_code(action) == Some(existing.key_code())
            })
            .map(HotkeyConflict::Configured)
    }

    pub const fn enable_autocomplete(self) -> bool {
        self.enable_autocomplete
    }

    pub const fn with_enable_autocomplete(mut self, enabled: bool) -> Self {
        self.enable_autocomplete = enabled;
        self
    }

    pub const fn enable_autocorrections(self) -> bool {
        self.enable_autocorrections
    }

    pub const fn with_enable_autocorrections(mut self, enabled: bool) -> Self {
        self.enable_autocorrections = enabled;
        self
    }

    pub const fn enable_auto_keyboard_switches(self) -> bool {
        self.enable_auto_keyboard_switches
    }

    pub const fn with_enable_auto_keyboard_switches(mut self, enabled: bool) -> Self {
        self.enable_auto_keyboard_switches = enabled;
        self
    }

    pub const fn notification_timeout_seconds(self) -> NotificationTimeoutSeconds {
        self.notification_timeout_seconds
    }

    pub const fn with_notification_timeout_seconds(
        mut self,
        timeout: NotificationTimeoutSeconds,
    ) -> Self {
        self.notification_timeout_seconds = timeout;
        self
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
    InvalidHotkeyKeyCode(u16),
    InvalidHotkeyEncoding,
    NotificationTimeoutOutOfRange(u32),
}

impl Display for SettingsError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClipboardHistoryLimitMustBePositive => {
                formatter.write_str("clipboard history limit must be positive")
            }
            Self::InvalidHotkeyKeyCode(value) => {
                write!(formatter, "invalid hotkey key code: {value}")
            }
            Self::InvalidHotkeyEncoding => formatter.write_str("invalid hotkey encoding"),
            Self::NotificationTimeoutOutOfRange(value) => write!(
                formatter,
                "notification timeout must be between {} and {} seconds, got {value}",
                NotificationTimeoutSeconds::MIN,
                NotificationTimeoutSeconds::MAX
            ),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoreWordOutcome {
    AddedToIgnoreList,
    AlreadyInIgnoreList,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgetWordOutcome {
    RemovedFromIgnoreList,
    RemovedFromUserWords,
    RemovedFromBoth,
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardEntryId(i64);

impl ClipboardEntryId {
    pub const fn get(self) -> i64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardEntryList {
    Current,
    Pinned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardReorderPosition {
    Before,
    After,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardEntryKind {
    Text,
    Image,
    Files,
}

impl ClipboardEntryKind {
    pub const fn as_stored(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Files => "files",
        }
    }

    fn try_from_stored(value: &str) -> Result<Self, DatabaseError> {
        match value {
            "text" => Ok(Self::Text),
            "image" => Ok(Self::Image),
            "files" => Ok(Self::Files),
            _ => Err(DatabaseError::InvalidStoredClipboardKind(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardEntryContent {
    Text(String),
    Image { format: String, data: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardEntryView {
    id: ClipboardEntryId,
    kind: ClipboardEntryKind,
    last_seen_at_ms: i64,
    copy_count: u32,
    pinned_at_ms: Option<i64>,
    content: ClipboardEntryContent,
}

impl ClipboardEntryView {
    pub const fn id(&self) -> ClipboardEntryId {
        self.id
    }

    pub const fn kind(&self) -> ClipboardEntryKind {
        self.kind
    }

    pub const fn last_seen_at_ms(&self) -> i64 {
        self.last_seen_at_ms
    }

    pub const fn copy_count(&self) -> u32 {
        self.copy_count
    }

    pub const fn pinned_at_ms(&self) -> Option<i64> {
        self.pinned_at_ms
    }

    pub const fn content(&self) -> &ClipboardEntryContent {
        &self.content
    }
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
    SequenceHistory(SequenceHistoryError),
    SchemaTooNew {
        found: i64,
        supported: i64,
    },
    InvalidStoredClipboardHistoryLimit(i64),
    InvalidStoredUndoHotkey(String),
    InvalidStoredHotkey {
        setting: &'static str,
        value: String,
    },
    InvalidStoredBoolean {
        setting: &'static str,
        value: i64,
    },
    InvalidStoredNotificationTimeout(i64),
    InvalidStoredDictionaryFrequency(i64),
    InvalidStoredUserWordUseCount(i64),
    InvalidStoredClipboardKind(String),
    InvalidStoredClipboardCopyCount(i64),
    InvalidClipboardImage,
    InvalidStoredLanguageCode(String),
    ClipboardEntryNotFound(i64),
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
            Self::SequenceHistory(error) => {
                write!(formatter, "invalid sequence history data: {error:?}")
            }
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
            Self::InvalidStoredHotkey { setting, value } => {
                write!(formatter, "invalid stored hotkey for {setting}: {value:?}")
            }
            Self::InvalidStoredBoolean { setting, value } => {
                write!(formatter, "invalid stored boolean for {setting}: {value}")
            }
            Self::InvalidStoredNotificationTimeout(value) => {
                write!(formatter, "invalid stored notification timeout: {value}")
            }
            Self::InvalidStoredDictionaryFrequency(value) => {
                write!(formatter, "invalid stored dictionary frequency: {value}")
            }
            Self::InvalidStoredUserWordUseCount(value) => {
                write!(formatter, "invalid stored user word use count: {value}")
            }
            Self::InvalidStoredClipboardKind(value) => {
                write!(formatter, "invalid stored clipboard kind: {value:?}")
            }
            Self::InvalidStoredClipboardCopyCount(value) => {
                write!(formatter, "invalid stored clipboard copy count: {value}")
            }
            Self::InvalidClipboardImage => formatter.write_str("invalid clipboard image"),
            Self::InvalidStoredLanguageCode(value) => {
                write!(formatter, "invalid stored language code: {value:?}")
            }
            Self::ClipboardEntryNotFound(id) => {
                write!(formatter, "clipboard entry {id} was not found")
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

impl From<SequenceHistoryError> for DatabaseError {
    fn from(error: SequenceHistoryError) -> Self {
        Self::SequenceHistory(error)
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
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let main_path: String = connection.query_row(
            "SELECT file FROM pragma_database_list WHERE name = 'main'",
            [],
            |row| row.get(0),
        )?;
        if !main_path.is_empty() {
            let journal_mode: String =
                connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
            if !journal_mode.eq_ignore_ascii_case("wal") {
                return Err(DatabaseError::Sqlite(rusqlite::Error::InvalidQuery));
            }
        }
        migrate(&mut connection)?;
        connection.execute_batch("PRAGMA synchronous = NORMAL;")?;
        Ok(Self { connection })
    }

    #[cfg(test)]
    pub(super) fn schema_version(&self) -> Result<i64, DatabaseError> {
        schema_version(&self.connection)
    }

    #[cfg(test)]
    pub(super) fn storage_pragmas(&self) -> Result<(String, i64), DatabaseError> {
        let journal_mode = self
            .connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        let synchronous = self
            .connection
            .pragma_query_value(None, "synchronous", |row| row.get(0))?;
        Ok((journal_mode, synchronous))
    }

    pub fn settings(&self) -> Result<AppSettings, DatabaseError> {
        let result = self.connection.query_row(
            r#"
            SELECT clipboard_history_limit, undo_hotkey, ignore_word_hotkey,
                   clipboard_current_hotkey, clipboard_pinned_hotkey,
                   completion_accept_hotkey, completion_next_word_hotkey,
                   enable_autocomplete, enable_autocorrections, enable_auto_keyboard_switches,
                   notification_timeout_seconds
            FROM app_settings WHERE singleton = 1
            "#,
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        );
        let (
            stored_limit,
            stored_undo_hotkey,
            stored_ignore_hotkey,
            stored_clipboard_current,
            stored_clipboard_pinned,
            stored_completion_accept,
            stored_completion_next,
            stored_autocomplete,
            stored_autocorrections,
            stored_auto_switches,
            stored_notification_timeout,
        ) = match result {
            Ok(value) => value,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Err(DatabaseError::MissingAppSettings);
            }
            Err(error) => return Err(DatabaseError::Sqlite(error)),
        };

        let mut settings = AppSettings::new(
            ClipboardHistoryLimit::try_from_stored(stored_limit)?,
            UndoHotkey::try_from_stored(&stored_undo_hotkey)?,
        );
        for (action, setting_name, stored) in [
            (
                HotkeyAction::IgnoreWord,
                "ignore_word_hotkey",
                stored_ignore_hotkey,
            ),
            (
                HotkeyAction::ClipboardCurrent,
                "clipboard_current_hotkey",
                stored_clipboard_current,
            ),
            (
                HotkeyAction::ClipboardPinned,
                "clipboard_pinned_hotkey",
                stored_clipboard_pinned,
            ),
            (
                HotkeyAction::CompletionAccept,
                "completion_accept_hotkey",
                stored_completion_accept,
            ),
            (
                HotkeyAction::CompletionNextWord,
                "completion_next_word_hotkey",
                stored_completion_next,
            ),
        ] {
            let binding = HotkeyBinding::try_from_stored(&stored).map_err(|_| {
                DatabaseError::InvalidStoredHotkey {
                    setting: setting_name,
                    value: stored.clone(),
                }
            })?;
            settings = settings.with_hotkey(action, binding);
        }
        let timeout = u32::try_from(stored_notification_timeout)
            .ok()
            .and_then(|value| NotificationTimeoutSeconds::try_new(value).ok())
            .ok_or(DatabaseError::InvalidStoredNotificationTimeout(
                stored_notification_timeout,
            ))?;
        Ok(settings
            .with_enable_autocomplete(stored_bool("enable_autocomplete", stored_autocomplete)?)
            .with_enable_autocorrections(stored_bool(
                "enable_autocorrections",
                stored_autocorrections,
            )?)
            .with_enable_auto_keyboard_switches(stored_bool(
                "enable_auto_keyboard_switches",
                stored_auto_switches,
            )?)
            .with_notification_timeout_seconds(timeout))
    }

    pub fn save_settings(&self, settings: AppSettings) -> Result<(), DatabaseError> {
        let changed = self.connection.execute(
            r#"
            UPDATE app_settings SET
                clipboard_history_limit = ?1,
                undo_hotkey = ?2,
                ignore_word_hotkey = ?3,
                clipboard_current_hotkey = ?4,
                clipboard_pinned_hotkey = ?5,
                completion_accept_hotkey = ?6,
                completion_next_word_hotkey = ?7,
                enable_autocomplete = ?8,
                enable_autocorrections = ?9,
                enable_auto_keyboard_switches = ?10,
                notification_timeout_seconds = ?11
            WHERE singleton = 1
            "#,
            params![
                i64::from(settings.clipboard_history_limit().get()),
                settings.hotkey(HotkeyAction::UndoOrForgetWord).as_stored(),
                settings.hotkey(HotkeyAction::IgnoreWord).as_stored(),
                settings.hotkey(HotkeyAction::ClipboardCurrent).as_stored(),
                settings.hotkey(HotkeyAction::ClipboardPinned).as_stored(),
                settings.hotkey(HotkeyAction::CompletionAccept).as_stored(),
                settings
                    .hotkey(HotkeyAction::CompletionNextWord)
                    .as_stored(),
                i64::from(settings.enable_autocomplete()),
                i64::from(settings.enable_autocorrections()),
                i64::from(settings.enable_auto_keyboard_switches()),
                i64::from(settings.notification_timeout_seconds().get()),
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
            .prepare("SELECT code FROM languages WHERE enabled = 1 ORDER BY id")?;
        let language_codes: Vec<String> = language_statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        drop(language_statement);

        let mut packs = Vec::with_capacity(language_codes.len());
        for code in language_codes {
            let id = LanguageId::try_new(code.clone())?;
            let Some(pack) = builtin_language_pack(&id)? else {
                return Err(DatabaseError::InvalidStoredLanguageCode(code));
            };
            packs.push(pack);
        }
        Ok(packs)
    }

    pub fn record_user_word(&self, term: &str, used_at_ms: i64) -> Result<UserWord, DatabaseError> {
        let input = UserWord::try_new(term, 1, used_at_ms)?;
        upsert_user_word(&self.connection, &input)
    }

    pub fn record_user_words_batch(
        &mut self,
        terms: &[String],
        used_at_ms: i64,
    ) -> Result<Vec<UserWord>, DatabaseError> {
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut updated = Vec::with_capacity(terms.len());
        for term in terms {
            let input = UserWord::try_new(term.as_str(), 1, used_at_ms)?;
            updated.push(upsert_user_word(&transaction, &input)?);
        }
        transaction.commit()?;
        Ok(updated)
    }

    pub fn delete_user_word(&self, term: &str) -> Result<bool, DatabaseError> {
        let normalized_term = normalize_word(term.trim());
        if normalized_term.is_empty() {
            return Ok(false);
        }
        Ok(self.connection.execute(
            "DELETE FROM user_words WHERE normalized_term = ?1",
            [normalized_term],
        )? != 0)
    }

    pub fn ignore_word(&mut self, term: &str) -> Result<IgnoreWordOutcome, DatabaseError> {
        let input = UserWord::try_new(term.trim(), 1, 0)?;
        let normalized_term = input.normalized_term().to_owned();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO ignored_words (normalized_term) VALUES (?1)",
            [&normalized_term],
        )? != 0;
        transaction.execute(
            "DELETE FROM user_words WHERE normalized_term = ?1",
            [&normalized_term],
        )?;
        transaction.commit()?;
        Ok(if inserted {
            IgnoreWordOutcome::AddedToIgnoreList
        } else {
            IgnoreWordOutcome::AlreadyInIgnoreList
        })
    }

    pub fn forget_word(&mut self, term: &str) -> Result<ForgetWordOutcome, DatabaseError> {
        let trimmed = term.trim();
        let normalized_term = normalize_word(trimmed);
        if normalized_term.is_empty()
            || trimmed.chars().any(char::is_whitespace)
            || !trimmed.chars().any(char::is_alphanumeric)
        {
            return Ok(ForgetWordOutcome::NotFound);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ignored_deleted = transaction.execute(
            "DELETE FROM ignored_words WHERE normalized_term = ?1",
            [&normalized_term],
        )? != 0;
        let user_deleted = transaction.execute(
            "DELETE FROM user_words WHERE normalized_term = ?1",
            [&normalized_term],
        )? != 0;
        transaction.commit()?;
        Ok(match (ignored_deleted, user_deleted) {
            (true, true) => ForgetWordOutcome::RemovedFromBoth,
            (true, false) => ForgetWordOutcome::RemovedFromIgnoreList,
            (false, true) => ForgetWordOutcome::RemovedFromUserWords,
            (false, false) => ForgetWordOutcome::NotFound,
        })
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
        transaction.execute(
            "DELETE FROM ignored_words WHERE normalized_term = ?1",
            [accepted.normalized_term()],
        )?;
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

    pub fn record_text_history(
        &self,
        text: &str,
        used_at_ms: i64,
    ) -> Result<SequenceCandidate, DatabaseError> {
        let input = SequenceCandidate::try_new(text, 1, used_at_ms)?;
        upsert_text_history(&self.connection, &input)
    }

    pub fn record_text_history_batch(
        &mut self,
        texts: &[String],
        used_at_ms: i64,
    ) -> Result<Vec<SequenceCandidate>, DatabaseError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut updated = Vec::with_capacity(texts.len());
        for text in texts {
            let input = SequenceCandidate::try_new(text.as_str(), 1, used_at_ms)?;
            updated.push(upsert_text_history(&transaction, &input)?);
        }
        transaction.commit()?;
        Ok(updated)
    }

    pub fn delete_text_history(&self, text: &str) -> Result<bool, DatabaseError> {
        Ok(self
            .connection
            .execute("DELETE FROM text_history WHERE text = ?1", [text])?
            != 0)
    }

    pub fn record_clipboard_text(
        &self,
        text: &str,
        observed_at_ms: i64,
    ) -> Result<ClipboardEntryView, DatabaseError> {
        validate_clipboard_text(text)?;
        let content_hash = clipboard_content_hash(ClipboardEntryKind::Text, text.as_bytes());
        let entry_id = self.connection.query_row(
            r#"
INSERT INTO clipboard_entries (
kind, last_seen_at_ms, content_hash, copy_count, current_order_index
)
VALUES (?1, ?2, ?3, 1, ?2)
ON CONFLICT(kind, content_hash) DO UPDATE SET
current_order_index = excluded.current_order_index,
copy_count = clipboard_entries.copy_count + 1
RETURNING id
"#,
            params![
                ClipboardEntryKind::Text.as_stored(),
                observed_at_ms,
                &content_hash
            ],
            |row| row.get::<_, i64>(0),
        )?;
        self.connection.execute(
            r#"
INSERT INTO clipboard_text (entry_id, text)
VALUES (?1, ?2)
ON CONFLICT(entry_id) DO UPDATE SET text = excluded.text
"#,
            params![entry_id, text],
        )?;
        self.load_clipboard_entry(ClipboardEntryId(entry_id))
    }

    pub fn record_clipboard_image(
        &self,
        format: &str,
        data: &[u8],
        observed_at_ms: i64,
    ) -> Result<ClipboardEntryView, DatabaseError> {
        validate_clipboard_image(format, data)?;
        let content_hash = clipboard_content_hash(ClipboardEntryKind::Image, data);
        let entry_id = self.connection.query_row(
            r#"
INSERT INTO clipboard_entries (
kind, last_seen_at_ms, content_hash, copy_count, current_order_index
)
VALUES (?1, ?2, ?3, 1, ?2)
ON CONFLICT(kind, content_hash) DO UPDATE SET
current_order_index = excluded.current_order_index,
copy_count = clipboard_entries.copy_count + 1
RETURNING id
"#,
            params![
                ClipboardEntryKind::Image.as_stored(),
                observed_at_ms,
                &content_hash
            ],
            |row| row.get::<_, i64>(0),
        )?;
        self.connection.execute(
            r#"
INSERT INTO clipboard_images (entry_id, format, data)
VALUES (?1, ?2, ?3)
ON CONFLICT(entry_id) DO UPDATE SET
format = excluded.format,
data = excluded.data
"#,
            params![entry_id, format, data],
        )?;
        self.load_clipboard_entry(ClipboardEntryId(entry_id))
    }

    pub fn mark_clipboard_entry_used(
        &self,
        entry_id: ClipboardEntryId,
        used_at_ms: i64,
    ) -> Result<ClipboardEntryView, DatabaseError> {
        self.connection.execute(
            r#"
UPDATE clipboard_entries
SET current_order_index = ?1,
copy_count = copy_count + 1
WHERE id = ?2
"#,
            params![used_at_ms, entry_id.get()],
        )?;
        self.load_clipboard_entry(entry_id)
    }

    pub fn pin_clipboard_entry(
        &self,
        entry_id: ClipboardEntryId,
        pinned_at_ms: i64,
    ) -> Result<ClipboardEntryView, DatabaseError> {
        self.connection.execute(
            r#"
UPDATE clipboard_entries
SET pinned_at_ms = COALESCE(pinned_at_ms, ?1),
pinned_order_index = COALESCE(pinned_order_index, ?1)
WHERE id = ?2
"#,
            params![pinned_at_ms, entry_id.get()],
        )?;
        self.load_clipboard_entry(entry_id)
    }

    pub fn unpin_clipboard_entry(
        &self,
        entry_id: ClipboardEntryId,
    ) -> Result<ClipboardEntryView, DatabaseError> {
        self.connection.execute(
"UPDATE clipboard_entries SET pinned_at_ms = NULL, pinned_order_index = NULL WHERE id = ?1",
[entry_id.get()],
)?;
        self.load_clipboard_entry(entry_id)
    }

    pub fn reorder_clipboard_entry(
        &mut self,
        list: ClipboardEntryList,
        moved_entry_id: ClipboardEntryId,
        target_entry_id: ClipboardEntryId,
        position: ClipboardReorderPosition,
    ) -> Result<(), DatabaseError> {
        if moved_entry_id == target_entry_id {
            return Ok(());
        }

        let (select_sql, update_sql, max_sql) = match list {
            ClipboardEntryList::Current => (
                "SELECT id FROM clipboard_entries WHERE kind IN ('text', 'image') ORDER BY current_order_index DESC, id DESC",
                "UPDATE clipboard_entries SET current_order_index = ?1 WHERE id = ?2",
                "SELECT MAX(current_order_index) FROM clipboard_entries WHERE kind IN ('text', 'image')",
            ),
            ClipboardEntryList::Pinned => (
                "SELECT id FROM clipboard_entries WHERE kind IN ('text', 'image') AND pinned_at_ms IS NOT NULL ORDER BY pinned_order_index DESC, id DESC",
                "UPDATE clipboard_entries SET pinned_order_index = ?1 WHERE id = ?2",
                "SELECT MAX(pinned_order_index) FROM clipboard_entries WHERE kind IN ('text', 'image') AND pinned_at_ms IS NOT NULL",
            ),
        };

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut statement = transaction.prepare(select_sql)?;
        let mut entry_ids = statement
            .query_map([], |row| row.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);

        let moved_id = moved_entry_id.get();
        let target_id = target_entry_id.get();
        let Some(moved_index) = entry_ids.iter().position(|id| *id == moved_id) else {
            return Err(DatabaseError::ClipboardEntryNotFound(moved_id));
        };
        if !entry_ids.contains(&target_id) {
            return Err(DatabaseError::ClipboardEntryNotFound(target_id));
        }

        let moved = entry_ids.remove(moved_index);
        let target_index = entry_ids
            .iter()
            .position(|id| *id == target_id)
            .expect("target was checked before moved entry removal");
        let insert_index = match position {
            ClipboardReorderPosition::Before => target_index,
            ClipboardReorderPosition::After => target_index + 1,
        };
        entry_ids.insert(insert_index, moved);

        let max_order = transaction
            .query_row(max_sql, [], |row| row.get::<_, Option<i64>>(0))?
            .unwrap_or(0);
        let base_order = max_order.max(entry_ids.len() as i64);
        let mut update = transaction.prepare(update_sql)?;
        for (index, entry_id) in entry_ids.iter().enumerate() {
            let order_value = base_order + (entry_ids.len() - index) as i64;
            update.execute(params![order_value, entry_id])?;
        }
        drop(update);
        transaction.commit()?;
        Ok(())
    }
    pub fn delete_clipboard_entry(
        &self,
        entry_id: ClipboardEntryId,
    ) -> Result<bool, DatabaseError> {
        Ok(self.connection.execute(
            "DELETE FROM clipboard_entries WHERE id = ?1",
            [entry_id.get()],
        )? != 0)
    }

    pub fn clear_clipboard_history(&self) -> Result<usize, DatabaseError> {
        Ok(self
            .connection
            .execute("DELETE FROM clipboard_entries", [])?)
    }

    pub fn prune_clipboard_history(
        &self,
        limit: ClipboardHistoryLimit,
    ) -> Result<usize, DatabaseError> {
        Ok(self.connection.execute(
            r#"
DELETE FROM clipboard_entries
WHERE pinned_at_ms IS NULL
AND id NOT IN (
SELECT id
FROM clipboard_entries
WHERE pinned_at_ms IS NULL
ORDER BY current_order_index DESC, id DESC
LIMIT ?1
)
"#,
            [i64::from(limit.get())],
        )?)
    }

    pub fn load_clipboard_current(
        &self,
        filter: &str,
        limit: usize,
    ) -> Result<Vec<ClipboardEntryView>, DatabaseError> {
        let mut statement = self.connection.prepare(
            r#"
SELECT clipboard_entries.id, clipboard_entries.kind, clipboard_entries.last_seen_at_ms,
clipboard_entries.copy_count, clipboard_entries.pinned_at_ms,
clipboard_text.text, clipboard_images.format, clipboard_images.data
FROM clipboard_entries
LEFT JOIN clipboard_text ON clipboard_text.entry_id = clipboard_entries.id
LEFT JOIN clipboard_images ON clipboard_images.entry_id = clipboard_entries.id
WHERE clipboard_entries.kind IN ('text', 'image')
ORDER BY clipboard_entries.current_order_index DESC, clipboard_entries.id DESC
"#,
        )?;
        let stored = statement
            .query_map([], stored_clipboard_row_from_sql)?
            .collect::<Result<Vec<_>, _>>()?;
        clipboard_entries_from_stored(stored, filter, limit)
    }

    pub fn load_clipboard_pinned(
        &self,
        filter: &str,
        limit: usize,
    ) -> Result<Vec<ClipboardEntryView>, DatabaseError> {
        let mut statement = self.connection.prepare(
            r#"
SELECT clipboard_entries.id, clipboard_entries.kind, clipboard_entries.last_seen_at_ms,
clipboard_entries.copy_count, clipboard_entries.pinned_at_ms,
clipboard_text.text, clipboard_images.format, clipboard_images.data
FROM clipboard_entries
LEFT JOIN clipboard_text ON clipboard_text.entry_id = clipboard_entries.id
LEFT JOIN clipboard_images ON clipboard_images.entry_id = clipboard_entries.id
WHERE clipboard_entries.kind IN ('text', 'image')
AND clipboard_entries.pinned_at_ms IS NOT NULL
ORDER BY clipboard_entries.pinned_order_index DESC, clipboard_entries.id DESC
"#,
        )?;
        let stored = statement
            .query_map([], stored_clipboard_row_from_sql)?
            .collect::<Result<Vec<_>, _>>()?;
        clipboard_entries_from_stored(stored, filter, limit)
    }

    fn load_clipboard_entry(
        &self,
        entry_id: ClipboardEntryId,
    ) -> Result<ClipboardEntryView, DatabaseError> {
        let result = self.connection.query_row(
            r#"
            SELECT clipboard_entries.id, clipboard_entries.kind, clipboard_entries.last_seen_at_ms,
                   clipboard_entries.copy_count, clipboard_entries.pinned_at_ms,
                   clipboard_text.text, clipboard_images.format, clipboard_images.data
            FROM clipboard_entries
            LEFT JOIN clipboard_text ON clipboard_text.entry_id = clipboard_entries.id
            LEFT JOIN clipboard_images ON clipboard_images.entry_id = clipboard_entries.id
            WHERE clipboard_entries.id = ?1 AND clipboard_entries.kind IN ('text', 'image')
            "#,
            [entry_id.get()],
            stored_clipboard_row_from_sql,
        );
        match result {
            Ok(stored) => clipboard_entry_from_stored(stored),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                Err(DatabaseError::ClipboardEntryNotFound(entry_id.get()))
            }
            Err(error) => Err(DatabaseError::Sqlite(error)),
        }
    }

    pub fn hide_completion_word(&self, normalized_term: &str) -> Result<(), DatabaseError> {
        let normalized_term = normalize_word(normalized_term);
        if normalized_term.is_empty() {
            return Ok(());
        }
        self.connection.execute(
            "INSERT OR IGNORE INTO completion_hidden_words (normalized_term) VALUES (?1)",
            [normalized_term],
        )?;
        Ok(())
    }

    pub fn load_hidden_completion_words(
        &self,
    ) -> Result<crate::completion::CompletionWordSuppressions, DatabaseError> {
        let mut statement = self.connection.prepare(
            "SELECT normalized_term FROM completion_hidden_words ORDER BY normalized_term",
        )?;
        let stored = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for normalized_term in &stored {
            if normalize_word(normalized_term) != *normalized_term {
                return Err(DatabaseError::InvalidStoredNormalizedTerm {
                    term: normalized_term.clone(),
                    normalized_term: normalized_term.clone(),
                });
            }
        }
        Ok(crate::completion::CompletionWordSuppressions::from_normalized_words(stored))
    }

    pub fn load_text_history(&self) -> Result<SequenceHistory, DatabaseError> {
        let mut statement = self
            .connection
            .prepare("SELECT text, use_count, last_used_at_ms FROM text_history ORDER BY text")?;
        let stored: Vec<(String, i64, i64)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<_, _>>()?;
        sequence_history_from_stored(stored)
    }

    pub(crate) fn load_active_text_history(
        &self,
        repeated_limit: usize,
        recent_singleton_limit: usize,
    ) -> Result<SequenceHistory, DatabaseError> {
        let repeated_limit = i64::try_from(repeated_limit).unwrap_or(i64::MAX);
        let recent_singleton_limit = i64::try_from(recent_singleton_limit).unwrap_or(i64::MAX);
        let mut statement = self.connection.prepare(
            r#"
            SELECT text, use_count, last_used_at_ms
            FROM (
                SELECT text, use_count, last_used_at_ms
                FROM text_history
                WHERE use_count > 1
                ORDER BY use_count DESC, last_used_at_ms DESC, text ASC
                LIMIT ?1
            )
            UNION ALL
            SELECT text, use_count, last_used_at_ms
            FROM (
                SELECT text, use_count, last_used_at_ms
                FROM text_history
                WHERE use_count = 1
                ORDER BY last_used_at_ms DESC, text ASC
                LIMIT ?2
            )
            "#,
        )?;
        let stored: Vec<(String, i64, i64)> = statement
            .query_map(params![repeated_limit, recent_singleton_limit], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<_, _>>()?;
        sequence_history_from_stored(stored)
    }

    // Correction history owns Undo state only; user vocabulary changes through user-word APIs.

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

    pub fn load_ignored_words(&self) -> Result<IgnoredWords, DatabaseError> {
        let mut statement = self
            .connection
            .prepare("SELECT normalized_term FROM ignored_words ORDER BY normalized_term")?;
        let stored = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for normalized_term in &stored {
            let validated = UserWord::try_new(normalized_term.clone(), 1, 0)?;
            if validated.normalized_term() != normalized_term {
                return Err(DatabaseError::InvalidStoredNormalizedTerm {
                    term: normalized_term.clone(),
                    normalized_term: normalized_term.clone(),
                });
            }
        }
        Ok(IgnoredWords::from_normalized_words(stored))
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

fn upsert_text_history(
    connection: &Connection,
    input: &SequenceCandidate,
) -> Result<SequenceCandidate, DatabaseError> {
    let stored = connection.query_row(
        r#"
INSERT INTO text_history (text, last_used_at_ms, use_count)
VALUES (?1, ?2, 1)
ON CONFLICT(text) DO UPDATE SET
    use_count = text_history.use_count + 1,
    last_used_at_ms = MAX(text_history.last_used_at_ms, excluded.last_used_at_ms)
RETURNING text, use_count, last_used_at_ms
"#,
        params![input.text(), input.last_used_at_ms()],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        },
    )?;
    sequence_candidate_from_stored(stored)
}

fn validate_clipboard_text(text: &str) -> Result<(), DatabaseError> {
    if text.is_empty() || text.contains('\0') {
        return Err(DatabaseError::InvalidCorrectionText);
    }
    Ok(())
}

fn validate_clipboard_image(format: &str, data: &[u8]) -> Result<(), DatabaseError> {
    if format.is_empty() || format.contains('\0') || data.is_empty() {
        return Err(DatabaseError::InvalidClipboardImage);
    }
    Ok(())
}

fn clipboard_content_hash(kind: ClipboardEntryKind, bytes: &[u8]) -> Vec<u8> {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in kind
        .as_stored()
        .bytes()
        .chain([0])
        .chain(bytes.iter().copied())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash.to_be_bytes().to_vec()
}

type StoredClipboardRow = (
    i64,
    String,
    i64,
    i64,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<Vec<u8>>,
);

fn stored_clipboard_row_from_sql(
    row: &rusqlite::Row<'_>,
) -> Result<StoredClipboardRow, rusqlite::Error> {
    Ok((
        row.get::<_, i64>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, i64>(2)?,
        row.get::<_, i64>(3)?,
        row.get::<_, Option<i64>>(4)?,
        row.get::<_, Option<String>>(5)?,
        row.get::<_, Option<String>>(6)?,
        row.get::<_, Option<Vec<u8>>>(7)?,
    ))
}

fn clipboard_entry_from_stored(
    stored: StoredClipboardRow,
) -> Result<ClipboardEntryView, DatabaseError> {
    let (
        id,
        stored_kind,
        last_seen_at_ms,
        stored_copy_count,
        pinned_at_ms,
        text,
        image_format,
        image_data,
    ) = stored;
    let kind = ClipboardEntryKind::try_from_stored(&stored_kind)?;
    let copy_count = u32::try_from(stored_copy_count)
        .map_err(|_| DatabaseError::InvalidStoredClipboardCopyCount(stored_copy_count))?;
    let content = match kind {
        ClipboardEntryKind::Text => {
            ClipboardEntryContent::Text(text.ok_or(DatabaseError::ClipboardEntryNotFound(id))?)
        }
        ClipboardEntryKind::Image => ClipboardEntryContent::Image {
            format: image_format.ok_or(DatabaseError::InvalidClipboardImage)?,
            data: image_data.ok_or(DatabaseError::InvalidClipboardImage)?,
        },
        ClipboardEntryKind::Files => {
            return Err(DatabaseError::InvalidStoredClipboardKind(stored_kind));
        }
    };
    Ok(ClipboardEntryView {
        id: ClipboardEntryId(id),
        kind,
        last_seen_at_ms,
        copy_count,
        pinned_at_ms,
        content,
    })
}

fn clipboard_entries_from_stored(
    stored: Vec<StoredClipboardRow>,
    filter: &str,
    limit: usize,
) -> Result<Vec<ClipboardEntryView>, DatabaseError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let normalized_filter = filter.trim().to_lowercase();
    let mut entries = Vec::new();
    for row in stored {
        let entry = clipboard_entry_from_stored(row)?;
        let matches_filter = normalized_filter.is_empty()
            || match entry.content() {
                ClipboardEntryContent::Text(text) => {
                    text.to_lowercase().contains(&normalized_filter)
                }
                ClipboardEntryContent::Image { format, .. } => {
                    "image".contains(&normalized_filter)
                        || format.to_lowercase().contains(&normalized_filter)
                }
            };
        if matches_filter {
            entries.push(entry);
            if entries.len() == limit {
                break;
            }
        }
    }
    Ok(entries)
}

fn sequence_history_from_stored(
    stored: Vec<(String, i64, i64)>,
) -> Result<SequenceHistory, DatabaseError> {
    let mut entries = Vec::with_capacity(stored.len());
    for row in stored {
        entries.push(sequence_candidate_from_stored(row)?);
    }
    Ok(SequenceHistory::new(entries))
}

fn sequence_candidate_from_stored(
    stored: (String, i64, i64),
) -> Result<SequenceCandidate, DatabaseError> {
    let (text, stored_use_count, last_used_at_ms) = stored;
    let use_count =
        u32::try_from(stored_use_count).map_err(|_| SequenceHistoryError::InvalidUseCount)?;
    Ok(SequenceCandidate::try_new(
        text,
        use_count,
        last_used_at_ms,
    )?)
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

fn sqlite_column_exists(
    connection: &Connection,
    table_name: &str,
    column_name: &str,
) -> Result<bool, DatabaseError> {
    let query = format!("PRAGMA table_info({table_name})");
    let mut statement = connection.prepare(&query)?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let stored_column_name: String = row.get(1)?;
        if stored_column_name == column_name {
            return Ok(true);
        }
    }
    Ok(false)
}

fn stored_bool(setting: &'static str, value: i64) -> Result<bool, DatabaseError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(DatabaseError::InvalidStoredBoolean { setting, value }),
    }
}

fn schema_version(connection: &Connection) -> Result<i64, DatabaseError> {
    Ok(connection.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

fn migrate(connection: &mut Connection) -> Result<(), DatabaseError> {
    let mut version = schema_version(connection)?;
    if version > CURRENT_SCHEMA_VERSION {
        return Err(DatabaseError::SchemaTooNew {
            found: version,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }

    if version < 1 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(SCHEMA_V1)?;
        transaction.pragma_update(None, "user_version", 1)?;
        transaction.commit()?;
        version = 1;
    }

    if version < 2 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(SCHEMA_V2)?;
        transaction.pragma_update(None, "user_version", 2)?;
        transaction.commit()?;
        version = 2;
    }

    if version < 3 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(SCHEMA_V3)?;
        transaction.pragma_update(None, "user_version", 3)?;
        transaction.commit()?;
        version = 3;
    }

    if version < 4 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(SCHEMA_V4)?;
        transaction.pragma_update(None, "user_version", 4)?;
        transaction.commit()?;
        version = 4;
    }

    if version < 5 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(SCHEMA_V5)?;
        transaction.pragma_update(None, "user_version", 5)?;
        transaction.commit()?;
        version = 5;
    }

    if version < 6 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !sqlite_column_exists(&transaction, "clipboard_entries", "current_order_index")? {
            transaction.execute(
                "ALTER TABLE clipboard_entries ADD COLUMN current_order_index INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !sqlite_column_exists(&transaction, "clipboard_entries", "pinned_order_index")? {
            transaction.execute(
                "ALTER TABLE clipboard_entries ADD COLUMN pinned_order_index INTEGER",
                [],
            )?;
        }
        transaction.execute_batch(SCHEMA_V6)?;
        transaction.pragma_update(None, "user_version", 6)?;
        transaction.commit()?;
        version = 6;
    }

    if version < 7 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(SCHEMA_V7)?;
        transaction.pragma_update(None, "user_version", 7)?;
        transaction.commit()?;
        version = 7;
    }

    if version < 8 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (column_name, statement) in APP_SETTINGS_V8_COLUMNS {
            if !sqlite_column_exists(&transaction, "app_settings", column_name)? {
                transaction.execute(statement, [])?;
            }
        }
        transaction.pragma_update(None, "user_version", 8)?;
        transaction.commit()?;
    }

    Ok(())
}
