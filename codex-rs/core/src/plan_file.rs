//! Plan file management for Plan Mode.
//!
//! This module handles the creation, resolution, and management of plan files
//! used during Plan Mode. Plan files are stored at `~/.codex/plans/{slug}.md`.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;

use dirs::home_dir;
use once_cell::sync::Lazy;
use tokio::fs;
use uuid::Uuid;

use crate::state::SessionState;

/// Cache of session ID to plan slug mappings.
static PLAN_SLUG_CACHE: Lazy<Mutex<HashMap<String, String>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

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

/// Generate or retrieve a unique slug for the session.
pub fn get_or_create_slug(session_id: &str) -> String {
    let mut cache = PLAN_SLUG_CACHE.lock().unwrap();

    if let Some(slug) = cache.get(session_id) {
        return slug.clone();
    }

    let plans_dir = get_plans_dir();
    let mut slug = String::new();

    // Try to find a unique slug
    for _ in 0..MAX_SLUG_ATTEMPTS {
        slug = Uuid::new_v4().to_string()[..8].to_string();
        let plan_path = plans_dir.join(format!("{}.md", slug));
        if !plan_path.exists() {
            break;
        }
    }

    cache.insert(session_id.to_string(), slug.clone());
    slug
}

/// Set a specific slug for a session (used when resuming from existing plan).
pub fn set_slug(session_id: &str, slug: &str) {
    let mut cache = PLAN_SLUG_CACHE.lock().unwrap();
    cache.insert(session_id.to_string(), slug.to_string());
}

/// Resolve the plan file path for a session.
pub fn resolve_plan_file_path(session_id: &str, agent_id: Option<&str>) -> PathBuf {
    let slug = get_or_create_slug(session_id);
    let plans_dir = get_plans_dir();

    match agent_id {
        Some(aid) if aid != session_id => {
            // Agent context: {slug}-agent-{agentId}.md
            plans_dir.join(format!("{}-agent-{}.md", slug, aid))
        }
        _ => {
            // Main session: {slug}.md
            plans_dir.join(format!("{}.md", slug))
        }
    }
}

/// Read plan content from file.
pub async fn extract_plan_from_file(session_id: &str, agent_id: Option<&str>) -> Option<String> {
    let path = resolve_plan_file_path(session_id, agent_id);
    fs::read_to_string(&path).await.ok()
}

/// Check if a given path is the plan file for the current session.
pub(crate) fn is_plan_file_path(path: &Path, session_state: &SessionState) -> bool {
    // Use the unified permission mode API
    if let Some(plan_file_path) = session_state.plan_file_path() {
        // Normalize both paths for comparison
        let normalized_path = path.canonicalize().ok().or_else(|| Some(path.to_path_buf()));
        let normalized_plan = Path::new(plan_file_path)
            .canonicalize()
            .ok()
            .or_else(|| Some(PathBuf::from(plan_file_path)));

        match (normalized_path, normalized_plan) {
            (Some(p1), Some(p2)) => p1 == p2,
            _ => path.to_string_lossy() == plan_file_path,
        }
    } else {
        false
    }
}

/// Check if a plan file exists for the session.
pub fn plan_exists(session_id: &str, agent_id: Option<&str>) -> bool {
    let path = resolve_plan_file_path(session_id, agent_id);
    path.exists()
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
    fn test_get_or_create_slug() {
        // Clear cache for test isolation
        {
            let mut cache = PLAN_SLUG_CACHE.lock().unwrap();
            cache.clear();
        }

        let session_id = "test-session-123";
        let slug1 = get_or_create_slug(session_id);
        let slug2 = get_or_create_slug(session_id);

        // Same session ID should return same slug
        assert_eq!(slug1, slug2);
        assert_eq!(slug1.len(), 8); // UUID first 8 chars
    }

    #[test]
    fn test_resolve_plan_file_path_main_session() {
        // Clear cache for test isolation
        {
            let mut cache = PLAN_SLUG_CACHE.lock().unwrap();
            cache.clear();
        }

        let session_id = "main-session";
        let path = resolve_plan_file_path(session_id, None);

        assert!(path.to_string_lossy().ends_with(".md"));
        assert!(!path.to_string_lossy().contains("-agent-"));
    }

    #[test]
    fn test_resolve_plan_file_path_agent() {
        // Clear cache for test isolation
        {
            let mut cache = PLAN_SLUG_CACHE.lock().unwrap();
            cache.clear();
        }

        let session_id = "main-session";
        let agent_id = "agent-456";
        let path = resolve_plan_file_path(session_id, Some(agent_id));

        assert!(path.to_string_lossy().ends_with(".md"));
        assert!(path.to_string_lossy().contains("-agent-"));
    }
}
