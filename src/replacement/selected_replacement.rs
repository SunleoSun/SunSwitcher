#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedText(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedTextError;

impl SelectedText {
    pub fn try_new(value: impl Into<String>) -> Result<Self, SelectedTextError> {
        let value = value.into();
        if value.is_empty() || value.contains('\0') {
            return Err(SelectedTextError);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedReplacementText(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedReplacementTextError;

impl SelectedReplacementText {
    pub fn try_new(value: impl Into<String>) -> Result<Self, SelectedReplacementTextError> {
        let value = value.into();
        if value.contains('\0') {
            return Err(SelectedReplacementTextError);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectedTextDecision {
    Keep,
    Replace(SelectedReplacementText),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedReplacementAction {
    source: SelectedText,
    replacement: SelectedReplacementText,
}

impl SelectedReplacementAction {
    pub fn source(&self) -> &SelectedText {
        &self.source
    }

    pub fn replacement(&self) -> &SelectedReplacementText {
        &self.replacement
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SelectedReplacementEngine;

impl SelectedReplacementEngine {
    pub fn new() -> Self {
        Self
    }

    pub fn plan(
        &self,
        selected: SelectedText,
        decision: SelectedTextDecision,
    ) -> Option<SelectedReplacementAction> {
        let SelectedTextDecision::Replace(replacement) = decision else {
            return None;
        };
        if selected.as_str() == replacement.as_str() {
            return None;
        }

        Some(SelectedReplacementAction {
            source: selected,
            replacement,
        })
    }
}
