//! Plan file management for Plan Mode.
//!
//! This module handles the creation, resolution, and management of plan files
//! used during Plan Mode. Plan files are stored at `~/.codex/plans/{slug}.md`.
//!
//! ## Slug Format
//!
//! Slugs use the memorable format `{adjective}-{verb}-{noun}`:
//! - Main session: `atomic-marinating-pumpkin.md`
//! - Sub-agent: `atomic-marinating-pumpkin-agent-{agentId}.md`
//!
//! ## Persistence
//!
//! The slug is stored in SessionModeContext and persisted with the session.
//! On resume, the slug is restored from the session state.

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
/// * `agent_id` - Optional agent ID for sub-agents
///
/// # Returns
/// - Main session: `~/.codex/plans/{slug}.md`
/// - Sub-agent: `~/.codex/plans/{slug}-agent-{agentId}.md`
pub fn resolve_plan_file_path_with_slug(slug: &str, agent_id: Option<&str>) -> PathBuf {
    let plans_dir = get_plans_dir();

    match agent_id {
        Some(aid) => {
            // Agent context: {slug}-agent-{agentId}.md
            plans_dir.join(format!("{slug}-agent-{aid}.md"))
        }
        None => {
            // Main session: {slug}.md
            plans_dir.join(format!("{slug}.md"))
        }
    }
}

/// Read plan content from file using a slug.
pub async fn extract_plan_from_file_with_slug(
    slug: &str,
    agent_id: Option<&str>,
) -> Option<String> {
    let path = resolve_plan_file_path_with_slug(slug, agent_id);
    fs::read_to_string(&path).await.ok()
}

/// Check if a plan file exists using a slug.
pub fn plan_exists_with_slug(slug: &str, agent_id: Option<&str>) -> bool {
    let path = resolve_plan_file_path_with_slug(slug, agent_id);
    path.exists()
}

/// Check if a given path is the plan file for the current session.
///
/// Uses the plan file path stored in SessionState when plan mode was entered.
pub(crate) fn is_plan_file_path(path: &Path, session_state: &SessionState) -> bool {
    if let Some(plan_file_path) = session_state.plan_file_path() {
        // Simple string comparison first (fast path)
        let path_str = path.to_string_lossy();
        if path_str == plan_file_path {
            return true;
        }

        // Try canonical paths for edge cases
        let normalized_path = path.canonicalize().ok();
        let normalized_plan = Path::new(plan_file_path).canonicalize().ok();

        match (normalized_path, normalized_plan) {
            (Some(p1), Some(p2)) => p1 == p2,
            _ => false,
        }
    } else {
        false
    }
}

// =============================================================================
// Deprecated: Legacy functions using global cache
// These are kept for backward compatibility but should be migrated away from.
// =============================================================================

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Mutex;

/// Legacy cache of session ID to plan slug mappings.
/// @deprecated Use session state's plan_slug field instead.
static PLAN_SLUG_CACHE: Lazy<Mutex<HashMap<String, String>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Legacy: Generate or retrieve a unique slug for the session.
/// @deprecated Use session.get_or_create_plan_slug() instead.
pub fn get_or_create_slug(session_id: &str) -> String {
    let mut cache = PLAN_SLUG_CACHE.lock().expect("lock poisoned");

    if let Some(slug) = cache.get(session_id) {
        return slug.clone();
    }

    let slug = generate_unique_slug();
    cache.insert(session_id.to_string(), slug.clone());
    slug
}

/// Legacy: Set a specific slug for a session.
/// @deprecated Use session.set_plan_slug() instead.
pub fn set_slug(session_id: &str, slug: &str) {
    let mut cache = PLAN_SLUG_CACHE.lock().expect("lock poisoned");
    cache.insert(session_id.to_string(), slug.to_string());
}

/// Legacy: Get the current slug for a session.
/// @deprecated Use session.get_plan_slug() instead.
pub fn get_slug(session_id: &str) -> Option<String> {
    let cache = PLAN_SLUG_CACHE.lock().expect("lock poisoned");
    cache.get(session_id).cloned()
}

/// Legacy: Resolve plan file path using session_id (looks up slug in global cache).
/// @deprecated Use resolve_plan_file_path_with_slug() with session.get_or_create_plan_slug().
pub fn resolve_plan_file_path(session_id: &str, agent_id: Option<&str>) -> PathBuf {
    let slug = get_or_create_slug(session_id);
    resolve_plan_file_path_with_slug(&slug, agent_id)
}

/// Legacy: Check if a plan file exists using session_id.
/// @deprecated Use plan_exists_with_slug() with session.get_plan_slug().
pub fn plan_exists(session_id: &str, agent_id: Option<&str>) -> bool {
    let slug = get_or_create_slug(session_id);
    plan_exists_with_slug(&slug, agent_id)
}

/// Legacy: Restore slug from a persisted value.
/// @deprecated Use session.set_plan_slug() instead.
pub fn restore_slug_from_persisted(session_id: &str, slug: &str) -> bool {
    set_slug(session_id, slug);
    plan_exists_with_slug(slug, None)
}

/// Clear the slug cache (useful for testing).
#[cfg(test)]
pub fn clear_slug_cache() {
    let mut cache = PLAN_SLUG_CACHE.lock().expect("lock poisoned");
    cache.clear();
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
        let parts: Vec<&str> = slug.split('-').collect();
        assert_eq!(parts.len(), 3, "Slug should have 3 parts: {slug}");
    }

    #[test]
    fn test_resolve_plan_file_path_with_slug_main() {
        let path = resolve_plan_file_path_with_slug("test-happy-slug", None);

        assert!(path.to_string_lossy().ends_with("test-happy-slug.md"));
        assert!(!path.to_string_lossy().contains("-agent-"));
    }

    #[test]
    fn test_resolve_plan_file_path_with_slug_agent() {
        let path = resolve_plan_file_path_with_slug("test-happy-slug", Some("agent-123"));

        assert!(path
            .to_string_lossy()
            .ends_with("test-happy-slug-agent-agent-123.md"));
    }

    // Legacy tests
    #[test]
    fn test_get_or_create_slug_cached() {
        clear_slug_cache();

        let session_id = "test-session-cached";
        let slug1 = get_or_create_slug(session_id);
        let slug2 = get_or_create_slug(session_id);

        // Same session ID should return same slug
        assert_eq!(slug1, slug2);
    }

    #[test]
    fn test_set_and_get_slug() {
        clear_slug_cache();

        let session_id = "test-session-set";
        assert!(get_slug(session_id).is_none());

        set_slug(session_id, "custom-test-slug");
        assert_eq!(get_slug(session_id), Some("custom-test-slug".to_string()));
    }
}
