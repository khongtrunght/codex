//! Model tier abstraction for generic model selection.
//!
//! This module provides a provider-agnostic way to select between different
//! model tiers (Default, Small) that map to user-configured model slugs.

use serde::Deserialize;
use serde::Serialize;
use std::fmt;
use std::str::FromStr;

/// Model tier for agent configuration.
/// Determines which configured model to use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelTier {
    /// Use the main configured model (config.model)
    #[default]
    Default,
    /// Use the small/fast model (config.small_model)
    Small,
    /// Inherit the parent's current model (may differ from config.model at runtime)
    Inherit,
}

impl FromStr for ModelTier {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "default" => Ok(ModelTier::Default),
            "small" => Ok(ModelTier::Small),
            "inherit" => Ok(ModelTier::Inherit),
            _ => Err(format!("Unknown model tier: {s}")),
        }
    }
}

impl fmt::Display for ModelTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelTier::Default => write!(f, "default"),
            ModelTier::Small => write!(f, "small"),
            ModelTier::Inherit => write!(f, "inherit"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_tier_from_str() {
        assert_eq!("default".parse::<ModelTier>().unwrap(), ModelTier::Default);
        assert_eq!("small".parse::<ModelTier>().unwrap(), ModelTier::Small);
        assert_eq!("inherit".parse::<ModelTier>().unwrap(), ModelTier::Inherit);
        assert_eq!("Default".parse::<ModelTier>().unwrap(), ModelTier::Default);
        assert_eq!("SMALL".parse::<ModelTier>().unwrap(), ModelTier::Small);
        assert_eq!("INHERIT".parse::<ModelTier>().unwrap(), ModelTier::Inherit);
        assert!("unknown".parse::<ModelTier>().is_err());
    }

    #[test]
    fn test_model_tier_display() {
        assert_eq!(ModelTier::Default.to_string(), "default");
        assert_eq!(ModelTier::Small.to_string(), "small");
        assert_eq!(ModelTier::Inherit.to_string(), "inherit");
    }

    #[test]
    fn test_model_tier_serde() {
        let tier = ModelTier::Small;
        let json = serde_json::to_string(&tier).unwrap();
        assert_eq!(json, "\"small\"");

        let parsed: ModelTier = serde_json::from_str("\"default\"").unwrap();
        assert_eq!(parsed, ModelTier::Default);
    }
}
