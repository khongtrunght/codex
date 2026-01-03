//! Plan mode system prompt attachments.
//!
//! Generates plan mode instructions that are injected into the conversation
//! when the model is in plan mode.
//!
//! This module provides:
//! - Multi-agent planning workflow with Task subagents for exploration and planning
//! - Plan mode reentry detection and instructions
//! - Sub-agent specific instructions

use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use serde::{Deserialize, Serialize};

use crate::plan_file::plan_exists;
use crate::tools::spec::ApplyToolConfig;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_protocol::openai_models::EditToolType;

/// Number of parallel Plan agents to use for multi-perspective planning.
const PLAN_AGENT_COUNT: usize = 3;

/// Number of parallel Explore agents to use for codebase research.
const EXPLORE_AGENT_COUNT: usize = 3;

/// Template for enhanced multi-agent plan mode instructions (main session).
const PLAN_MODE_ENHANCED_TEMPLATE: &str = r#"Plan mode is active. The user indicated that they do not want you to execute yet -- you MUST NOT make any edits (with the exception of the plan file mentioned below), run any non-readonly tools (including changing configs or making commits), or otherwise make any changes to the system. This supercedes any other instructions you have received.

## Plan File Info:
{plan_file_info}
You should build your plan incrementally by writing to or editing this file. NOTE that this is the only file you are allowed to edit - other than this you are only allowed to take READ-ONLY actions.

**Plan File Guidelines:** The plan file should contain only your final recommended approach, not all alternatives considered. Keep it comprehensive yet concise - detailed enough to execute effectively while avoiding unnecessary verbosity.

## Enhanced Planning Workflow

### Phase 1: Initial Understanding
Goal: Gain a comprehensive understanding of the user's request by reading through code and asking them questions. Critical: In this phase you should only use the Explore subagent type.

1. Understand the user's request thoroughly

2. **Launch up to {explore_agent_count} Explore agents IN PARALLEL** (single message, multiple tool calls) to efficiently explore the codebase. Each agent can focus on different aspects:
   - Example: One agent searches for existing implementations, another explores related components, a third investigates testing patterns
   - Provide each agent with a specific search focus or area to explore
   - Quality over quantity - {explore_agent_count} agents maximum, but you should try to use the minimum number of agents necessary (usually just 1)
   - Use 1 agent when: the task is isolated to known files, the user provided specific file paths, or you're making a small targeted change. Use multiple agents when: the scope is uncertain, multiple areas of the codebase are involved, or you need to understand existing patterns before planning.
   - Take into account any context you already have from the user's request or from the conversation so far when deciding how many agents to launch

3. Use {ask_user_question_tool} tool to clarify ambiguities in the user request up front.

### Phase 2: Multi-Agent Planning
Goal: Come up with different approaches to solve the problem identified in phase 1 by launching multiple Plan subagent types.
Launch **up to {plan_agent_count}** {task_tool} agents IN PARALLEL (single message, multiple tool calls) with Plan subagent type, based on task complexity.

**Quality over quantity**:
- Provide each agent with a perspective on how to approach the design process.
- Simple tasks may need fewer agents (minimum 1), where as complex tasks benefit from multiple perspectives (up to {plan_agent_count}). If the task is simple, you should try to use the minimum number of agents necessary (usually just 1)
- Focus on meaningful contrasts between perspectives. Quality of agent perspectives is more important than quantity

Dynamically generate perspectives based on the task. Examples:
- For a new feature: simplicity vs performance vs maintainability vs existing patterns
- For a bug fix: root cause vs workaround vs prevention vs testing
- For refactoring: minimal change vs clean architecture vs gradual migration vs full rewrite

In each agent prompt:
- Describe the specific perspective/approach to take
- Provide any background context that may help the agent with their task without prescribing the exact design itself
- Request a detailed plan from their perspective

### Phase 3: Synthesis
Goal: Synthesize the perspectives from Phase 2, and ensure that it aligns with the users's intentions by asking them questions.
1. Collect all agent responses
2. Each agent will return an implementation plan along with a list of critical files that should be read. You should keep these in mind and read them before you start implementing the plan
3. Use {ask_user_question_tool} to ask the users questions about trade offs.

### Phase 4: Final Plan
Once you are have all the information you need, ensure that the plan file has been updated with your synthesized recommendation including:
- Recommended approach with rationale
- Key insights from different perspectives
- Critical files that need modification

### Phase 5: Call {exit_plan_mode_tool}
At the very end of your turn, once you have asked the user questions and are happy with your final plan file - you should always call {exit_plan_mode_tool} to indicate to the user that you are done planning.
This is critical - your turn should only end with either asking the user a question or calling {exit_plan_mode_tool}. Do not stop unless it's for these 2 reasons.

NOTE: At any point in time through this workflow you should feel free to ask the user questions or clarifications. Don't make large assumptions about user intent. The goal is to present a well researched plan to the user, and tie any loose ends before implementation begins."#;

/// Template for sub-agent plan mode instructions.
const PLAN_MODE_SUBAGENT_TEMPLATE: &str = r#"Plan mode is active. The user indicated that they do not want you to execute yet -- you MUST NOT make any edits, run any non-readonly tools (including changing configs or making commits), or otherwise make any changes to the system. This supercedes any other instructions you have received (for example, to make edits). Instead, you should:

## Plan File Info:
{plan_file_info}
You should build your plan incrementally by writing to or editing this file. NOTE that this is the only file you are allowed to edit - other than this you are only allowed to take READ-ONLY actions.
Answer the user's query comprehensively, using the {ask_user_question_tool} tool if you need to ask the user clarifying questions. If you do use the {ask_user_question_tool}, make sure to ask all clarifying questions you need to fully understand the user's intent before proceeding."#;

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
pub struct PlanModeInstructions {
    pub plan_file_path: String,
    pub contents: String,
    /// Whether this is a reentry (returning to plan mode after exiting).
    #[serde(default)]
    pub is_reentry: bool,
}

impl From<PlanModeInstructions> for ResponseItem {
    fn from(pmi: PlanModeInstructions) -> Self {
        // Use <system-reminder> tags to wrap the content
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: format!(
                    "<system-reminder>\n{}\n</system-reminder>",
                    pmi.contents
                ),
            }],
        }
    }
}

/// Generate plan mode system message for main session with multi-agent workflow.
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

    let contents = PLAN_MODE_ENHANCED_TEMPLATE
        .replace("{plan_file_info}", &plan_file_info)
        .replace("{plan_agent_count}", &PLAN_AGENT_COUNT.to_string())
        .replace("{explore_agent_count}", &EXPLORE_AGENT_COUNT.to_string())
        .apply_tool_config(edit_tool, shell_tool);

    PlanModeInstructions {
        plan_file_path: plan_file_path.to_string(),
        contents,
        is_reentry: false,
    }
}

/// Generate plan mode system message for sub-agent context.
pub fn generate_plan_mode_subagent_attachment(
    plan_file_path: &str,
    plan_file_exists: bool,
    edit_tool: Option<EditToolType>,
    shell_tool: ConfigShellToolType,
) -> PlanModeInstructions {
    let plan_file_info = if plan_file_exists {
        format!(
            "A plan file already exists at {}. You can read it and make incremental edits using the {{edit_tool}} tool if you need to.",
            plan_file_path
        )
    } else {
        format!(
            "No plan file exists yet. You should create your plan at {} using the {{write_tool}} tool if you need to.",
            plan_file_path
        )
    };

    let contents = PLAN_MODE_SUBAGENT_TEMPLATE
        .replace("{plan_file_info}", &plan_file_info)
        .apply_tool_config(edit_tool, shell_tool);

    PlanModeInstructions {
        plan_file_path: plan_file_path.to_string(),
        contents,
        is_reentry: false,
    }
}

/// Generate plan mode reentry message.
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
        is_reentry: true,
    }
}

/// Configuration for collecting plan mode attachments.
#[derive(Debug, Clone)]
pub struct PlanModeAttachmentConfig {
    pub session_id: String,
    pub plan_file_path: String,
    pub is_subagent: bool,
    pub has_exited_plan_mode: bool,
    pub edit_tool: Option<EditToolType>,
    pub shell_tool: ConfigShellToolType,
}

/// Collect plan mode attachments based on session state.
///
/// Returns a vector of ResponseItems to be injected into the conversation.
/// The vector may contain:
/// - A reentry attachment if has_exited_plan_mode is true and plan exists
/// - A plan mode attachment (either main session or sub-agent variant)
pub fn collect_plan_mode_attachments(config: &PlanModeAttachmentConfig) -> Vec<ResponseItem> {
    let mut attachments = Vec::new();

    // Check for reentry condition: previously exited plan mode AND plan file exists
    let plan_file_exists = plan_exists(&config.session_id, None);
    if config.has_exited_plan_mode && plan_file_exists {
        // Add reentry attachment
        let reentry = generate_plan_mode_reentry_attachment(
            &config.plan_file_path,
            config.edit_tool.clone(),
            config.shell_tool.clone(),
        );
        attachments.push(ResponseItem::from(reentry));
    }

    // Add the appropriate plan mode attachment
    if config.is_subagent {
        let subagent = generate_plan_mode_subagent_attachment(
            &config.plan_file_path,
            plan_file_exists,
            config.edit_tool.clone(),
            config.shell_tool.clone(),
        );
        attachments.push(ResponseItem::from(subagent));
    } else {
        let main = generate_plan_mode_attachment(
            &config.session_id,
            &config.plan_file_path,
            config.edit_tool.clone(),
            config.shell_tool.clone(),
        );
        attachments.push(ResponseItem::from(main));
    }

    attachments
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
        // Verify multi-agent workflow elements
        assert!(attachment.contents.contains("Phase 1: Initial Understanding"));
        assert!(attachment.contents.contains("Phase 2: Multi-Agent Planning"));
        assert!(attachment.contents.contains("Explore agents"));
        assert!(attachment.contents.contains("Plan subagent"));
        // Verify placeholders are replaced
        assert!(attachment.contents.contains("write_file")); // from {write_tool}
        assert!(attachment.contents.contains("ask_user_question")); // from {ask_user_question_tool}
        assert!(attachment.contents.contains("exit_plan_mode")); // from {exit_plan_mode_tool}
        assert!(attachment.contents.contains("task")); // from {task_tool}
        assert!(!attachment.is_reentry);
    }

    #[test]
    fn test_generate_plan_mode_subagent_attachment() {
        let attachment = generate_plan_mode_subagent_attachment(
            "/home/user/.codex/plans/abc123.md",
            true,
            Some(EditToolType::FileEdit),
            ConfigShellToolType::Bash,
        );

        assert!(attachment.contents.contains("Plan mode is active"));
        assert!(attachment.contents.contains("Answer the user's query comprehensively"));
        assert!(!attachment.contents.contains("Phase 1")); // Sub-agent doesn't get multi-phase workflow
        assert!(!attachment.is_reentry);
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
        assert!(attachment.is_reentry);
    }

    #[test]
    fn test_collect_plan_mode_attachments_normal() {
        let config = PlanModeAttachmentConfig {
            session_id: "test-session".to_string(),
            plan_file_path: "/tmp/plan.md".to_string(),
            is_subagent: false,
            has_exited_plan_mode: false,
            edit_tool: Some(EditToolType::FileEdit),
            shell_tool: ConfigShellToolType::Bash,
        };

        let attachments = collect_plan_mode_attachments(&config);
        // Should only have the main plan mode attachment (no reentry)
        assert_eq!(attachments.len(), 1);
    }

    #[test]
    fn test_collect_plan_mode_attachments_subagent() {
        let config = PlanModeAttachmentConfig {
            session_id: "test-session".to_string(),
            plan_file_path: "/tmp/plan.md".to_string(),
            is_subagent: true,
            has_exited_plan_mode: false,
            edit_tool: Some(EditToolType::FileEdit),
            shell_tool: ConfigShellToolType::Bash,
        };

        let attachments = collect_plan_mode_attachments(&config);
        assert_eq!(attachments.len(), 1);
    }

    #[test]
    fn test_plan_mode_instructions_to_response_item() {
        let instructions = PlanModeInstructions {
            plan_file_path: "/tmp/plan.md".to_string(),
            contents: "Test contents".to_string(),
            is_reentry: false,
        };

        let item: ResponseItem = instructions.into();
        match item {
            ResponseItem::Message { role, content, .. } => {
                assert_eq!(role, "user");
                assert!(!content.is_empty());
                if let ContentItem::InputText { text } = &content[0] {
                    assert!(text.contains("<system-reminder>"));
                    assert!(text.contains("Test contents"));
                    assert!(text.contains("</system-reminder>"));
                }
            }
            _ => panic!("Expected Message variant"),
        }
    }

    #[test]
    fn test_reentry_instructions_to_response_item() {
        let instructions = PlanModeInstructions {
            plan_file_path: "/tmp/plan.md".to_string(),
            contents: "Reentry contents".to_string(),
            is_reentry: true,
        };

        let item: ResponseItem = instructions.into();
        match item {
            ResponseItem::Message { content, .. } => {
                if let ContentItem::InputText { text } = &content[0] {
                    // Both reentry and normal use <system-reminder> tags
                    assert!(text.contains("<system-reminder>"));
                    assert!(text.contains("Reentry contents"));
                }
            }
            _ => panic!("Expected Message variant"),
        }
    }
}
