//! Plan file management for Plan Mode.
//!
//! This module handles the creation, resolution, and management of plan files
//! used during Plan Mode. Plan files are stored at `~/.codex/plans/{slug}.md`.
//!
//! ## Slug Format
//!
//! Slugs use the memorable format `{adjective}-{verb}-{noun}`:
//! - Example: `atomic-marinating-pumpkin.md`
//!
//! ## Persistence
//!
//! The slug is stored in SessionState and persisted with the session.
//! On resume, the slug is restored from the session state.
//!
//! Note: Subagents do not create plan files - only the main session manages plans.

use std::path::Path;
use std::path::PathBuf;

use dirs::home_dir;
use tokio::fs;

use crate::plan_slug::generate_slug;
use crate::state::SessionState;

/// Maximum attempts to generate a unique slug.
const MAX_SLUG_ATTEMPTS: usize = 10;

/// Get the plans directory path (~/.codex/plans/).
pub fn get_plans_dir() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
        .join("plans")
}

/// Ensure plans directory exists.
pub async fn ensure_plans_dir() -> std::io::Result<()> {
    let dir = get_plans_dir();
    if !dir.exists() {
        fs::create_dir_all(&dir).await?;
    }
    Ok(())
}

/// Generate a unique slug that doesn't conflict with existing plan files.
///
/// Uses memorable three-word format: `{adjective}-{verb}-{noun}`
/// Example: "atomic-marinating-pumpkin"
pub fn generate_unique_slug() -> String {
    let plans_dir = get_plans_dir();

    for _ in 0..MAX_SLUG_ATTEMPTS {
        let slug = generate_slug();
        let plan_path = plans_dir.join(format!("{slug}.md"));
        if !plan_path.exists() {
            return slug;
        }
    }

    // Fallback: if all attempts collide (extremely unlikely), just use the last one
    generate_slug()
}

/// Resolve the plan file path using a slug.
///
/// # Arguments
/// * `slug` - The memorable slug (e.g., "atomic-marinating-pumpkin")
///
/// # Returns
/// Path: `~/.codex/plans/{slug}.md`
pub fn resolve_plan_file_path_with_slug(slug: &str) -> PathBuf {
    let plans_dir = get_plans_dir();
    plans_dir.join(format!("{slug}.md"))
}

/// Read plan content from file using a slug.
pub async fn extract_plan_from_file_with_slug(slug: &str) -> Option<String> {
    let path = resolve_plan_file_path_with_slug(slug);
    fs::read_to_string(&path).await.ok()
}

/// Check if a plan file exists using a slug.
pub fn plan_exists_with_slug(slug: &str) -> bool {
    let path = resolve_plan_file_path_with_slug(slug);
    path.exists()
}

/// Check if a given path is the plan file for the current session.
///
/// Derives the plan file path from the plan slug stored in SessionState.
pub(crate) fn is_plan_file_path(path: &Path, session_state: &SessionState) -> bool {
    if let Some(slug) = session_state.plan_slug() {
        let plan_file_path = resolve_plan_file_path_with_slug(slug);

        // Simple path comparison first (fast path)
        if path == plan_file_path {
            return true;
        }

        // Try canonical paths for edge cases
        let normalized_path = path.canonicalize().ok();
        let normalized_plan = plan_file_path.canonicalize().ok();

        match (normalized_path, normalized_plan) {
            (Some(p1), Some(p2)) => p1 == p2,
            _ => false,
        }
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_plans_dir() {
        let dir = get_plans_dir();
        assert!(dir.ends_with("plans"));
        assert!(dir.to_string_lossy().contains(".codex"));
    }

    #[test]
    fn test_generate_unique_slug_format() {
        let slug = generate_unique_slug();

        // Should be in format: adjective-verb-noun
        assert_eq!(
            slug.split('-').count(),
            3,
            "Slug should have 3 parts: {slug}"
        );
    }

    #[test]
    fn test_resolve_plan_file_path_with_slug() {
        let path = resolve_plan_file_path_with_slug("test-happy-slug");

        assert!(path.to_string_lossy().ends_with("test-happy-slug.md"));
    }
}
