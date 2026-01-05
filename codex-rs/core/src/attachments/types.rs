//! Attachment type definitions.
//!
//! All attachment variants are defined here with their data and conversion logic.

use codex_protocol::models::AttachmentData;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_protocol::openai_models::EditToolType;

use crate::tools::spec::ApplyToolConfig;

/// Number of parallel Plan agents to use for multi-perspective planning.
const PLAN_AGENT_COUNT: usize = 3;

/// Number of parallel Explore agents to use for codebase research.
const EXPLORE_AGENT_COUNT: usize = 3;

/// Turns between attachment injections (matching JS TURNS_BETWEEN_ATTACHMENTS).
pub const TURNS_BETWEEN_ATTACHMENTS: usize = 5;

/// Tools configuration needed for attachment rendering.
#[derive(Debug, Clone)]
pub struct ToolsConfig {
    pub edit_tool: Option<EditToolType>,
    pub shell_tool: ConfigShellToolType,
}

/// Convert AttachmentData to ResponseItem::Message items for API call.
/// This follows the GhostSnapshot pattern: Attachment is stored in history,
/// then expanded to Message items during get_history_for_prompt().
pub fn attachment_data_to_messages(data: &AttachmentData, config: &ToolsConfig) -> Vec<ResponseItem> {
    match data {
        AttachmentData::PlanMode {
            plan_file_path,
            is_subagent,
            plan_exists,
        } => {
            #[cfg(feature = "exit_plan_mode")]
            {
                if *is_subagent {
                    generate_subagent_items(plan_file_path, *plan_exists, config)
                } else {
                    generate_main_session_items(plan_file_path, *plan_exists, config)
                }
            }
            #[cfg(not(feature = "exit_plan_mode"))]
            {
                generate_legacy_plan_mode_items(config, *is_subagent)
            }
        }
        AttachmentData::PlanModeReentry { plan_file_path } => {
            generate_reentry_items(plan_file_path, config)
        }
    }
}

/// Expand Attachment items in history to Message items (GhostSnapshot pattern).
/// Called during get_history_for_prompt() to convert Attachment markers to actual messages.
pub fn expand_attachments(items: Vec<ResponseItem>, config: &ToolsConfig) -> Vec<ResponseItem> {
    items
        .into_iter()
        .flat_map(|item| match item {
            ResponseItem::Attachment { ref data, .. } => {
                // Expand to Message items
                attachment_data_to_messages(data, config)
            }
            other => vec![other],
        })
        .collect()
}

// =============================================================================
// Plan Mode Templates
// =============================================================================

fn plan_agent_count_from_env_or_defaults() -> usize {
    std::env::var("CODEX_PLAN_V2_AGENT_COUNT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&v| (1..=10).contains(&v))
        .unwrap_or(PLAN_AGENT_COUNT)
}

fn explore_agent_count_from_env_or_default() -> usize {
    std::env::var("CLAUDE_CODE_PLAN_V2_EXPLORE_AGENT_COUNT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&v| (1..=10).contains(&v))
        .unwrap_or(EXPLORE_AGENT_COUNT)
}

// =============================================================================
// Generation Functions
// =============================================================================

fn generate_main_session_items(
    plan_file_path: &str,
    plan_exists: bool,
    config: &ToolsConfig,
) -> Vec<ResponseItem> {
    /// Template for enhanced multi-agent plan mode instructions (main session).
    const PLAN_MODE_ENHANCED_TEMPLATE: &str = r#"Plan mode is active. The user indicated that they do not want you to execute yet -- you MUST NOT make any edits (with the exception of the plan file mentioned below), run any non-readonly tools (including changing configs or making commits), or otherwise make any changes to the system. This supercedes any other instructions you have received.

## Plan File Info:
{plan_file_info}
You should build your plan incrementally by writing to or editing this file. NOTE that this is the only file you are allowed to edit - other than this you are only allowed to take READ-ONLY actions.

**Plan File Guidelines:** The plan file should contain only your final recommended approach, not all alternatives considered. Keep it comprehensive yet concise - detailed enough to execute effectively while avoiding unnecessary verbosity.

## Enhanced Planning Workflow

### Phase 1: Initial Understanding
Goal: Gain a comprehensive understanding of the user's request by reading through code and asking them questions. Critical: In this phase you should only use the {explore_agent_type} subagent type.

1. Understand the user's request thoroughly

2. **Launch up to {explore_agent_count} {explore_agent_type} agents IN PARALLEL** (single message, multiple tool calls) to efficiently explore the codebase. Each agent can focus on different aspects:
   - Example: One agent searches for existing implementations, another explores related components, a third investigates testing patterns
   - Provide each agent with a specific search focus or area to explore
   - Quality over quantity - {explore_agent_count} agents maximum, but you should try to use the minimum number of agents necessary (usually just 1)
   - Use 1 agent when: the task is isolated to known files, the user provided specific file paths, or you're making a small targeted change. Use multiple agents when: the scope is uncertain, multiple areas of the codebase are involved, or you need to understand existing patterns before planning.
   - Take into account any context you already have from the user's request or from the conversation so far when deciding how many agents to launch

3. Use {ask_user_question_tool} tool to clarify ambiguities in the user request up front.

{phase_2}

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

    let plan_file_info = if plan_exists {
        format!(
            "A plan file already exists at {plan_file_path}. You can read it and make incremental edits using the {{edit_tool}} tool."
        )
    } else {
        format!(
            "No plan file exists yet. You should create your plan at {plan_file_path} using the {{write_tool}} tool."
        )
    };

    let num_planing_agent = plan_agent_count_from_env_or_defaults();

    let phase_2_prompt = if num_planing_agent > 1 {
        r#"### Phase 2: Multi-Agent Planning
Goal: Come up with different approaches to solve the problem identified in phase 1 by launching multiple {plan_agent_type} subagent types.
Launch **up to {plan_agent_count}** {task_tool} agents IN PARALLEL (single message, multiple tool calls) with {plan_agent_type} subagent type, based on task complexity.

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
"#
    } else {
        r#"### Phase 2: Planning
Goal: Come up with an approach to solve the problem identified in phase 1 by launching a {plan_type} subagent.

In the agent prompt:
- Provide any background context that may help the agent with their task without prescribing the exact design itself
- Request a detailed plan"#
    };

    let contents = PLAN_MODE_ENHANCED_TEMPLATE
        .replace("{phase_2}", phase_2_prompt)
        .replace("{plan_file_info}", &plan_file_info)
        .replace("{plan_agent_count}", &num_planing_agent.to_string())
        .replace(
            "{explore_agent_count}",
            &explore_agent_count_from_env_or_default().to_string(),
        )
        .replace("{plan_agent_type}", "plan")
        .replace("{explore_agent_type}", "explore")
        .apply_tool_config(config.edit_tool.clone(), config.shell_tool);

    vec![wrap_in_system_reminder(&contents)]
}

#[allow(dead_code)]
fn generate_legacy_plan_mode_items(config: &ToolsConfig, is_subagent: bool) -> Vec<ResponseItem> {
    const PLAN_MODE_LEGACY_TEMPLATE: &str = r#"Plan mode is active. The user indicated that they do not want you to execute yet -- you MUST NOT make any edits, run any non-readonly tools (including changing configs or making commits), or otherwise make any changes to the system. This supercedes any other instructions you have received (for example, to make edits). Instead, you should:
1. Answer the user's query comprehensively, using the {ask_user_question_tool} tool if you need to ask the user clarifying questions. If you do use the {{ask_user_question_tool}}, make sure to ask all clarifying questions you need to fully understand the user's intent before proceeding.{agent_usage_requirement}
2. When you're done researching, present your plan by calling the {confirm_plan_tool} tool, which will prompt the user to confirm the plan. Do NOT make any file changes or run any tools that modify the system state in any way until the user has confirmed the plan."#;
    let agent_usage_requirement = if !is_subagent {
        " You MUST use a single {task_tool} tool call with {agent_type} subagent type to gather information. Even if you have already started researching directly, you must immediately switch to using an agent instead."
    } else {
        ""
    };

    let contents = PLAN_MODE_LEGACY_TEMPLATE
        .replace("{agent_usage_requirement}", agent_usage_requirement)
        .replace("{agent_type}", "plan")
        .apply_tool_config(config.edit_tool.clone(), config.shell_tool);

    vec![wrap_in_system_reminder(&contents)]
}

fn generate_subagent_items(
    plan_file_path: &str,
    plan_exists: bool,
    config: &ToolsConfig,
) -> Vec<ResponseItem> {
    let plan_file_info = if plan_exists {
        format!(
            "A plan file already exists at {plan_file_path}. You can read it and make incremental edits using the {{edit_tool}} tool if you need to."
        )
    } else {
        format!(
            "No plan file exists yet. You should create your plan at {plan_file_path} using the {{write_tool}} tool if you need to."
        )
    };

    /// Template for sub-agent plan mode instructions.
    const PLAN_MODE_SUBAGENT_TEMPLATE: &str = r#"Plan mode is active. The user indicated that they do not want you to execute yet -- you MUST NOT make any edits, run any non-readonly tools (including changing configs or making commits), or otherwise make any changes to the system. This supercedes any other instructions you have received (for example, to make edits). Instead, you should:

## Plan File Info:
{plan_file_info}
You should build your plan incrementally by writing to or editing this file. NOTE that this is the only file you are allowed to edit - other than this you are only allowed to take READ-ONLY actions.
Answer the user's query comprehensively, using the {ask_user_question_tool} tool if you need to ask the user clarifying questions. If you do use the {ask_user_question_tool}, make sure to ask all clarifying questions you need to fully understand the user's intent before proceeding."#;

    let contents = PLAN_MODE_SUBAGENT_TEMPLATE
        .replace("{plan_file_info}", &plan_file_info)
        .apply_tool_config(config.edit_tool.clone(), config.shell_tool);

    vec![wrap_in_system_reminder(&contents)]
}

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

fn generate_reentry_items(plan_file_path: &str, config: &ToolsConfig) -> Vec<ResponseItem> {
    let contents = PLAN_MODE_REENTRY_TEMPLATE
        .replace("{plan_file_path}", plan_file_path)
        .apply_tool_config(config.edit_tool.clone(), config.shell_tool);

    vec![wrap_in_system_reminder(&contents)]
}

/// Wrap content in <system-reminder> tags as a user message.
fn wrap_in_system_reminder(contents: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: format!("<system-reminder>\n{contents}\n</system-reminder>"),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_mode_main_session() {
        let data = AttachmentData::PlanMode {
            plan_file_path: "/tmp/plan.md".to_string(),
            is_subagent: false,
            plan_exists: false,
        };

        let config = ToolsConfig {
            edit_tool: Some(EditToolType::FileEdit),
            shell_tool: ConfigShellToolType::Bash,
        };

        let items = attachment_data_to_messages(&data, &config);
        assert_eq!(items.len(), 1);

        if let ResponseItem::Message { content, .. } = &items[0] {
            if let ContentItem::InputText { text } = &content[0] {
                assert!(text.contains("<system-reminder>"));
                assert!(text.contains("Plan mode is active"));
                assert!(text.contains("Phase 1: Initial Understanding"));
                assert!(text.contains("/tmp/plan.md"));
            }
        }
    }

    #[test]
    fn test_plan_mode_subagent() {
        let data = AttachmentData::PlanMode {
            plan_file_path: "/tmp/plan.md".to_string(),
            is_subagent: true,
            plan_exists: true,
        };

        let config = ToolsConfig {
            edit_tool: Some(EditToolType::FileEdit),
            shell_tool: ConfigShellToolType::Bash,
        };

        let items = attachment_data_to_messages(&data, &config);
        assert_eq!(items.len(), 1);

        if let ResponseItem::Message { content, .. } = &items[0] {
            if let ContentItem::InputText { text } = &content[0] {
                assert!(text.contains("Answer the user's query comprehensively"));
                assert!(!text.contains("Phase 1")); // Subagent doesn't get phases
            }
        }
    }

    #[test]
    fn test_plan_mode_reentry() {
        let data = AttachmentData::PlanModeReentry {
            plan_file_path: "/tmp/plan.md".to_string(),
        };

        let config = ToolsConfig {
            edit_tool: Some(EditToolType::FileEdit),
            shell_tool: ConfigShellToolType::Bash,
        };

        let items = attachment_data_to_messages(&data, &config);
        assert_eq!(items.len(), 1);

        if let ResponseItem::Message { content, .. } = &items[0] {
            if let ContentItem::InputText { text } = &content[0] {
                assert!(text.contains("Re-entering Plan Mode"));
                assert!(text.contains("/tmp/plan.md"));
            }
        }
    }

    #[test]
    fn test_expand_attachments() {
        let config = ToolsConfig {
            edit_tool: Some(EditToolType::FileEdit),
            shell_tool: ConfigShellToolType::Bash,
        };

        // Create history with Attachment item
        let history = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "test".to_string(),
                }],
            },
            ResponseItem::Attachment {
                data: AttachmentData::PlanMode {
                    plan_file_path: "/tmp/plan.md".to_string(),
                    is_subagent: false,
                    plan_exists: false,
                },
                timestamp: "2024-01-01T00:00:00Z".to_string(),
            },
        ];

        // Expand attachments
        let expanded = expand_attachments(history, &config);

        // Should have 2 items: original user message + expanded plan mode message
        assert_eq!(expanded.len(), 2);

        // First should be original user message
        assert!(matches!(&expanded[0], ResponseItem::Message { role, .. } if role == "user"));

        // Second should be expanded plan mode (NOT Attachment)
        assert!(matches!(&expanded[1], ResponseItem::Message { .. }));
        assert!(!matches!(&expanded[1], ResponseItem::Attachment { .. }));
    }
}
