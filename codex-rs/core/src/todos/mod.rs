//! Plan/Todo persistence module.
//!
//! Persists plans to `{codex_home}/todos/{session_id}.json` so they survive
//! session resume.

use codex_protocol::plan_tool::UpdatePlanArgs;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Subdirectory within codex_home where plan files are stored.
pub const TODOS_SUBDIR: &str = "todos";

/// Returns the directory where plan files are stored.
pub fn get_todo_dir(codex_home: &Path) -> PathBuf {
    codex_home.join(TODOS_SUBDIR)
}

/// Returns the path to the plan file for a given session/agent.
///
/// - For main session: `{codex_home}/todos/{session_id}.json`
/// - For subagent: `{codex_home}/todos/{session_id}-agent-{agent_id}.json`
pub fn get_todo_path(codex_home: &Path, session_id: &str, agent_id: Option<&str>) -> PathBuf {
    let filename = match agent_id {
        Some(aid) => format!("{session_id}-agent-{aid}.json"),
        None => format!("{session_id}.json"),
    };
    get_todo_dir(codex_home).join(filename)
}

/// Saves a plan to disk.
///
/// Creates the todos directory if it doesn't exist.
pub fn save_plan(
    codex_home: &Path,
    session_id: &str,
    agent_id: Option<&str>,
    plan: &UpdatePlanArgs,
) -> io::Result<()> {
    let dir = get_todo_dir(codex_home);
    fs::create_dir_all(&dir)?;
    let path = get_todo_path(codex_home, session_id, agent_id);
    let json = serde_json::to_string_pretty(plan)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fs::write(path, json)
}

/// Loads a plan from disk.
///
/// Returns `Ok(None)` if the file doesn't exist.
/// Returns `Ok(None)` and logs a warning if the file exists but can't be parsed.
pub fn load_plan(
    codex_home: &Path,
    session_id: &str,
    agent_id: Option<&str>,
) -> io::Result<Option<UpdatePlanArgs>> {
    let path = get_todo_path(codex_home, session_id, agent_id);
    match fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(plan) => Ok(Some(plan)),
            Err(e) => {
                tracing::warn!("Failed to parse plan file {:?}: {}", path, e);
                Ok(None)
            }
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::plan_tool::{PlanItemArg, StepStatus};
    use tempfile::tempdir;

    #[test]
    fn test_get_todo_path_main_session() {
        let codex_home = PathBuf::from("/home/user/.codex");
        let path = get_todo_path(&codex_home, "session-123", None);
        assert_eq!(
            path,
            PathBuf::from("/home/user/.codex/todos/session-123.json")
        );
    }

    #[test]
    fn test_get_todo_path_subagent() {
        let codex_home = PathBuf::from("/home/user/.codex");
        let path = get_todo_path(&codex_home, "session-123", Some("agent-456"));
        assert_eq!(
            path,
            PathBuf::from("/home/user/.codex/todos/session-123-agent-agent-456.json")
        );
    }

    #[test]
    fn test_save_and_load_plan() {
        let temp_dir = tempdir().unwrap();
        let codex_home = temp_dir.path();

        let plan = UpdatePlanArgs {
            explanation: Some("Test plan".to_string()),
            plan: vec![
                PlanItemArg {
                    step: "Step 1".to_string(),
                    status: StepStatus::Completed,
                },
                PlanItemArg {
                    step: "Step 2".to_string(),
                    status: StepStatus::InProgress,
                },
            ],
        };

        // Save
        save_plan(codex_home, "test-session", None, &plan).unwrap();

        // Load
        let loaded = load_plan(codex_home, "test-session", None).unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.explanation, Some("Test plan".to_string()));
        assert_eq!(loaded.plan.len(), 2);
        assert_eq!(loaded.plan[0].step, "Step 1");
        assert_eq!(loaded.plan[0].status, StepStatus::Completed);
    }

    #[test]
    fn test_load_nonexistent_plan() {
        let temp_dir = tempdir().unwrap();
        let codex_home = temp_dir.path();

        let loaded = load_plan(codex_home, "nonexistent", None).unwrap();
        assert!(loaded.is_none());
    }
}
