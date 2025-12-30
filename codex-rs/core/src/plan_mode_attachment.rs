//! Plan mode system prompt attachments.
//!
//! Generates plan mode instructions that are injected into the conversation
//! when the model enters plan mode. Matches Claude Code's uS3 and mS3 functions.

use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use serde::{Deserialize, Serialize};

use crate::plan_file::plan_exists;
use crate::tools::spec::ApplyToolConfig;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_protocol::openai_models::EditToolType;

/// Template for plan mode instructions.
/// Uses placeholders that are replaced via ApplyToolConfig.
const PLAN_MODE_TEMPLATE: &str = r#"Plan mode is active. The user indicated that they do not want you to execute yet -- you MUST NOT make any edits (with the exception of the plan file mentioned below), run any non-readonly tools (including changing configs or making commits), or otherwise make any changes to the system. This supercedes any other instructions you have received.

## Plan File Info:
{plan_file_info}
You should build your plan incrementally by writing to or editing this file. NOTE that this is the only file you are allowed to edit - other than this you are only allowed to take READ-ONLY actions.

**Plan File Guidelines:** The plan file should contain only your final recommended approach, not all alternatives considered. Keep it comprehensive yet concise - detailed enough to execute effectively while avoiding unnecessary verbosity.

## Planning Workflow

1. **Understand the Request**: Read through code and ask clarifying questions using {ask_user_question_tool} tool.

2. **Explore the Codebase**: Use read-only tools ({read_tool}, {glob_tool}, {grep_tool}) to understand existing patterns and architecture.

3. **Design Your Approach**: Consider trade-offs and architectural decisions.

4. **Write Your Plan**: Create/update the plan file with your recommended approach including:
   - Recommended approach with rationale
   - Key files that need modification
   - Step-by-step implementation strategy

5. **Call {exit_plan_mode_tool}**: When your plan is complete and ready for user approval, call {exit_plan_mode_tool}.

NOTE: At any point in time through this workflow you should feel free to ask the user questions or clarifications. Don't make large assumptions about user intent. The goal is to present a well researched plan to the user, and tie any loose ends before implementation begins."#;

/// Template for plan mode reentry instructions.
const PLAN_MODE_REENTRY_TEMPLATE: &str = r#"## Re-entering Plan Mode

You are returning to plan mode after having previously exited it. A plan file exists at {plan_file_path} from your previous planning session.

**Before proceeding with any new planning, you should:**
1. Read the existing plan file to understand what was previously planned
2. Evaluate the user's current request against that plan
3. Decide how to proceed:
   - **Different task**: If the user's request is for a different task—even if it's similar or related—start fresh by overwriting the existing plan
   - **Same task, continuing**: If this is explicitly a continuation or refinement of the exact same task, modify the existing plan while cleaning up outdated or irrelevant sections
4. Continue on with the plan process and most importantly you should always edit the plan file one way or the other before calling {exit_plan_mode_tool}

Treat this as a fresh planning session. Do not assume the existing plan is relevant without evaluating it first."#;

/// Plan mode instructions that can be converted to a ResponseItem.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename = "plan_mode_instructions", rename_all = "snake_case")]
pub(crate) struct PlanModeInstructions {
    pub plan_file_path: String,
    pub contents: String,
}

impl From<PlanModeInstructions> for ResponseItem {
    fn from(pmi: PlanModeInstructions) -> Self {
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: format!(
                    "<plan_mode>\n<plan_file>{}</plan_file>\n{}\n</plan_mode>",
                    pmi.plan_file_path, pmi.contents
                ),
            }],
        }
    }
}

/// Generate plan mode system message for main session.
/// Matches Claude Code's uS3 function.
pub fn generate_plan_mode_attachment(
    session_id: &str,
    plan_file_path: &str,
    edit_tool: Option<EditToolType>,
    shell_tool: ConfigShellToolType,
) -> PlanModeInstructions {
    let plan_file_exists = plan_exists(session_id, None);

    let plan_file_info = if plan_file_exists {
        format!(
            "A plan file already exists at {}. You can read it and make incremental edits using the {{edit_tool}} tool.",
            plan_file_path
        )
    } else {
        format!(
            "No plan file exists yet. You should create your plan at {} using the {{write_tool}} tool.",
            plan_file_path
        )
    };

    let contents = PLAN_MODE_TEMPLATE
        .replace("{plan_file_info}", &plan_file_info)
        .apply_tool_config(edit_tool, shell_tool);

    PlanModeInstructions {
        plan_file_path: plan_file_path.to_string(),
        contents,
    }
}

/// Generate plan mode reentry message.
/// Matches Claude Code's plan_mode_reentry attachment.
pub fn generate_plan_mode_reentry_attachment(
    plan_file_path: &str,
    edit_tool: Option<EditToolType>,
    shell_tool: ConfigShellToolType,
) -> PlanModeInstructions {
    let contents = PLAN_MODE_REENTRY_TEMPLATE
        .replace("{plan_file_path}", plan_file_path)
        .apply_tool_config(edit_tool, shell_tool);

    PlanModeInstructions {
        plan_file_path: plan_file_path.to_string(),
        contents,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_plan_mode_attachment_new_file() {
        let attachment = generate_plan_mode_attachment(
            "test-session",
            "/home/user/.codex/plans/abc123.md",
            Some(EditToolType::FileEdit),
            ConfigShellToolType::Bash,
        );

        assert!(attachment.contents.contains("Plan mode is active"));
        assert!(attachment.contents.contains("No plan file exists yet"));
        assert!(attachment.contents.contains("/home/user/.codex/plans/abc123.md"));
        // Verify placeholders are replaced
        assert!(attachment.contents.contains("write_file"));
        assert!(attachment.contents.contains("read_file"));
        assert!(attachment.contents.contains("exit_plan_mode"));
    }

    #[test]
    fn test_generate_plan_mode_reentry_attachment() {
        let attachment = generate_plan_mode_reentry_attachment(
            "/home/user/.codex/plans/abc123.md",
            Some(EditToolType::FileEdit),
            ConfigShellToolType::Bash,
        );

        assert!(attachment.contents.contains("Re-entering Plan Mode"));
        assert!(attachment.contents.contains("/home/user/.codex/plans/abc123.md"));
        assert!(attachment.contents.contains("exit_plan_mode"));
    }
}
