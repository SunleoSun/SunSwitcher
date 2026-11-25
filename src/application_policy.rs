use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::correction::CorrectionFeaturePolicy;
use crate::persistence::AppSettings;

/// A lexical, case-insensitive full Windows executable path, not a basename or PID.
/// No filesystem/process lookup is needed to construct or compare this identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ApplicationIdentity(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidApplicationIdentity;

impl Display for InvalidApplicationIdentity {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("expected a full Windows executable path (for example C:\\Apps\\app.exe)")
    }
}

impl Error for InvalidApplicationIdentity {}

impl ApplicationIdentity {
    pub fn try_new(path: &str) -> Result<Self, InvalidApplicationIdentity> {
        let path = path.replace('/', "\\").to_lowercase();
        let path = if let Some(unc) = path.strip_prefix("\\\\?\\unc\\") {
            format!("\\\\{unc}")
        } else {
            path.strip_prefix("\\\\?\\").unwrap_or(&path).to_owned()
        };
        let bytes = path.as_bytes();
        let tail = if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && &bytes[1..3] == b":\\" {
            &path[3..]
        } else if let Some(unc) = path.strip_prefix("\\\\") {
            // UNC identities require server, share, and an executable path.
            if unc.split('\\').count() < 3 {
                return Err(InvalidApplicationIdentity);
            }
            unc
        } else {
            return Err(InvalidApplicationIdentity);
        };
        if tail.split('\\').any(|part| {
            part.is_empty()
                || matches!(part, "." | "..")
                || part.ends_with([' ', '.'])
                || part
                    .chars()
                    .any(|c| c.is_control() || "<>:\"|?*".contains(c))
        }) {
            return Err(InvalidApplicationIdentity);
        }
        Ok(Self(path))
    }

    pub fn executable_path(&self) -> &str {
        &self.0
    }

    pub fn file_name(&self) -> &str {
        self.0
            .rsplit('\\')
            .next()
            .expect("validated executable path")
    }
}

pub fn global_correction_policy(settings: AppSettings) -> CorrectionFeaturePolicy {
    CorrectionFeaturePolicy::new(
        settings.enable_autocorrections(),
        settings.enable_auto_keyboard_switches(),
    )
}

/// Immutable runtime projection of the suppression-only persisted relation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplicationOverrides(BTreeSet<ApplicationIdentity>);

impl ApplicationOverrides {
    pub fn from_applications(applications: impl IntoIterator<Item = ApplicationIdentity>) -> Self {
        Self(applications.into_iter().collect())
    }

    pub fn applications(&self) -> impl Iterator<Item = &ApplicationIdentity> {
        self.0.iter()
    }

    pub fn suppresses(&self, application: &ApplicationIdentity) -> bool {
        self.0.contains(application)
    }

    pub fn suppresses_input(&self, foreground: Option<&ApplicationIdentity>) -> bool {
        foreground.is_some_and(|application| self.suppresses(application))
    }

    /// Unknown foreground ownership fails closed for automatic edits only.
    pub fn effective_policy(
        &self,
        settings: AppSettings,
        foreground: Option<&ApplicationIdentity>,
    ) -> CorrectionFeaturePolicy {
        if foreground.is_none_or(|application| self.suppresses(application)) {
            CorrectionFeaturePolicy::new(false, false)
        } else {
            global_correction_policy(settings)
        }
    }
}

#[cfg(test)]
#[path = "application_policy_certification.rs"]
mod certification;
