pub mod adaptive;
pub mod correction;
pub mod input;
pub mod language;
pub mod lexicon;
pub mod persistence;
pub mod replacement;

#[cfg(target_os = "windows")]
pub mod windows;
