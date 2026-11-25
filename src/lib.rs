pub mod adaptive;
pub mod application_policy;
pub mod completion;
pub mod correction;
pub mod diagnostics;
pub mod input;
pub mod language;
pub mod lexicon;
pub mod persistence;
pub mod replacement;
pub mod settings;

#[cfg(test)]
#[path = "composition_certifications/application_completion_policy_certification.rs"]
mod application_completion_policy_certification;

#[cfg(test)]
#[path = "composition_certifications/persistence_completion_retention_certification.rs"]
mod persistence_completion_retention_certification;

#[cfg(target_os = "windows")]
pub mod windows;
