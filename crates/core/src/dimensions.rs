//! The two list-backed classification dimensions: `stack` and `stage`.
//! `category` stays the single-valued "domain"; these hold zero or more values
//! from the editable `stacks` and `stages` lists in `box.toml`.
use crate::{BoxConfig, Error, Result};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Stack,
    Stage,
}

impl Dimension {
    pub fn key(self) -> &'static str {
        match self {
            Self::Stack => "stack",
            Self::Stage => "stage",
        }
    }
    /// A value outside the configured list.
    pub fn invalid_value_code(self) -> &'static str {
        match self {
            Self::Stack => "INVALID_STACK",
            Self::Stage => "INVALID_STAGE",
        }
    }
    /// A malformed list in `box.toml` or in a settings request.
    pub fn invalid_list_code(self) -> &'static str {
        match self {
            Self::Stack => "INVALID_STACKS",
            Self::Stage => "INVALID_STAGES",
        }
    }
    /// A settings change that would orphan a value some skill still holds.
    pub fn in_use_code(self) -> &'static str {
        match self {
            Self::Stack => "STACK_IN_USE",
            Self::Stage => "STAGE_IN_USE",
        }
    }
    pub fn allowed(self, config: &BoxConfig) -> &[String] {
        match self {
            Self::Stack => &config.stacks,
            Self::Stage => &config.stages,
        }
    }
}

/// Reject every value that is not literally in the configured list.
pub fn validate(dimension: Dimension, values: &[String], allowed: &[String]) -> Result<()> {
    match values.iter().find(|value| !allowed.contains(value)) {
        None => Ok(()),
        Some(value) => Err(Error::new(
            dimension.invalid_value_code(),
            format!(
                "{}: '{value}' is not in the {}s list of box.toml (allowed: {})",
                dimension.key(),
                dimension.key(),
                allowed.join(", ")
            ),
        )),
    }
}

/// Configured-list order without duplicates. Values outside the list keep
/// their relative order at the end, so validation can still name them.
pub fn ordered(values: &[String], allowed: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut result: Vec<&String> = values.iter().filter(|value| seen.insert(*value)).collect();
    result.sort_by_key(|value| {
        allowed
            .iter()
            .position(|candidate| candidate == *value)
            .unwrap_or(usize::MAX)
    });
    result.into_iter().cloned().collect()
}

/// List entries follow the category name rules, except that a list may be
/// empty: a skill needs one category but any number of dimension values.
pub fn validate_list(dimension: Dimension, list: &[String]) -> Result<()> {
    if list
        .iter()
        .any(|s| s.trim() != s || s.is_empty() || s.chars().any(char::is_control))
        || list.iter().collect::<BTreeSet<_>>().len() != list.len()
    {
        return Err(Error::new(
            dimension.invalid_list_code(),
            format!(
                "The {}s list must contain unique, non-empty names without surrounding spaces",
                dimension.key()
            ),
        ));
    }
    Ok(())
}

pub fn validate_lists(config: &BoxConfig) -> Result<()> {
    validate_list(Dimension::Stack, &config.stacks)?;
    validate_list(Dimension::Stage, &config.stages)
}
