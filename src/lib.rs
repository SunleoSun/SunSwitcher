pub mod adaptive;
pub mod completion;
pub mod correction;
pub mod diagnostics;
pub mod input;
pub mod language;
pub mod lexicon;
pub mod persistence;
pub mod replacement;
pub mod settings;

#[cfg(target_os = "windows")]
pub mod windows;
