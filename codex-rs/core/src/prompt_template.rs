//! Prompt template rendering using Askama.
//!
//! This module provides type-safe template rendering for prompts,
//! using Askama for compile-time validation.

use askama::Template;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_protocol::openai_models::EditToolType;

use crate::tools::spec::APPLY_PATCH_TOOL_NAME;
use crate::tools::spec::ASK_USER_QUESTION_TOOL_NAME;
use crate::tools::spec::BASH_TOOL_NAME;
use crate::tools::spec::EDIT_FILE_TOOL_NAME;
use crate::tools::spec::ENTER_PLAN_MODE_TOOL_NAME;
use crate::tools::spec::EXEC_COMMAND_TOOL_NAME;
use crate::tools::spec::EXIT_PLAN_MODE_TOOL_NAME;
use crate::tools::spec::GLOB_TOOL_NAME;
use crate::tools::spec::GREP_FILES_TOOL_NAME;
use crate::tools::spec::READ_FILE_TOOL_NAME;
use crate::tools::spec::SHELL_COMMAND_TOOL_NAME;
use crate::tools::spec::SHELL_TOOL_NAME;
use crate::tools::spec::TASK_TOOL_NAME;
use crate::tools::spec::TODO_WRITE_TOOL_NAME;
use crate::tools::spec::WRITE_FILE_TOOL_NAME;

/// Tool configuration context for Askama templates.
///
/// Stores the original enum types and provides helper methods for templates.
/// Use `{{ tools.shell_tool_name() }}` in templates to get the tool name string.
#[derive(Clone, Debug)]
pub struct ToolConfig {
    pub edit_tool: Option<EditToolType>,
    pub shell_tool: ConfigShellToolType,
}

impl ToolConfig {
    pub fn new(edit_tool: Option<EditToolType>, shell_tool: ConfigShellToolType) -> Self {
        Self {
            edit_tool,
            shell_tool,
        }
    }

    // === Configuration-dependent tool names ===

    /// Returns the shell tool name as a string for use in templates.
    pub fn shell_tool_name(&self) -> &'static str {
        match self.shell_tool {
            ConfigShellToolType::Bash => BASH_TOOL_NAME,
            ConfigShellToolType::Local | ConfigShellToolType::Default => SHELL_TOOL_NAME,
            ConfigShellToolType::ShellCommand => SHELL_COMMAND_TOOL_NAME,
            ConfigShellToolType::UnifiedExec => EXEC_COMMAND_TOOL_NAME,
            ConfigShellToolType::Disabled => SHELL_TOOL_NAME,
        }
    }

    /// Returns the edit tool name as a string for use in templates.
    pub fn edit_tool_name(&self) -> &'static str {
        match self.edit_tool {
            Some(EditToolType::ApplyPatchFreeform) | Some(EditToolType::ApplyPatchFunction) => {
                APPLY_PATCH_TOOL_NAME
            }
            Some(EditToolType::FileEdit) | None => EDIT_FILE_TOOL_NAME,
        }
    }

    /// Returns the write tool name as a string for use in templates.
    pub fn write_tool_name(&self) -> &'static str {
        match self.edit_tool {
            Some(EditToolType::ApplyPatchFreeform) | Some(EditToolType::ApplyPatchFunction) => {
                APPLY_PATCH_TOOL_NAME
            }
            Some(EditToolType::FileEdit) | None => WRITE_FILE_TOOL_NAME,
        }
    }

    /// Returns true if the edit tool is apply_patch (for conditional sections).
    pub fn is_apply_patch(&self) -> bool {
        matches!(
            self.edit_tool,
            Some(EditToolType::ApplyPatchFreeform) | Some(EditToolType::ApplyPatchFunction)
        )
    }

    // === Fixed tool names (not configuration-dependent) ===

    pub fn glob_tool(&self) -> &'static str {
        GLOB_TOOL_NAME
    }

    pub fn grep_tool(&self) -> &'static str {
        GREP_FILES_TOOL_NAME
    }

    pub fn read_tool(&self) -> &'static str {
        READ_FILE_TOOL_NAME
    }

    pub fn task_tool(&self) -> &'static str {
        TASK_TOOL_NAME
    }

    pub fn todo_write_tool(&self) -> &'static str {
        TODO_WRITE_TOOL_NAME
    }

    pub fn ask_user_question_tool(&self) -> &'static str {
        ASK_USER_QUESTION_TOOL_NAME
    }

    pub fn enter_plan_mode_tool(&self) -> &'static str {
        ENTER_PLAN_MODE_TOOL_NAME
    }

    pub fn exit_plan_mode_tool(&self) -> &'static str {
        EXIT_PLAN_MODE_TOOL_NAME
    }
}

/// Template for the general main prompt.
///
/// This prompt is used for general-purpose agent interactions.
#[derive(Template)]
#[template(path = "general_main_prompt.md")]
pub struct GeneralMainPrompt {
    pub tools: ToolConfig,
}

// =============================================================================
// Agent Prompts (external templates)
// =============================================================================

/// Template for the explore agent prompt.
#[derive(Template)]
#[template(path = "agents/explore_agent_prompt.md")]
pub struct ExploreAgentPrompt {
    pub tools: ToolConfig,
}

/// Template for the plan sub-agent prompt.
#[derive(Template)]
#[template(path = "agents/plan_subagent_prompt.md")]
pub struct PlanSubagentPrompt {
    pub tools: ToolConfig,
}

// =============================================================================
// Plan Mode Prompts (external templates)
// =============================================================================

/// Template for enhanced multi-agent plan mode instructions (main session).
#[derive(Template)]
#[template(path = "plan_mode/enhanced.md")]
pub struct PlanModeEnhancedPrompt {
    pub tools: ToolConfig,
    pub plan_file_info: String,
    pub plan_agent_count: usize,
    pub explore_agent_count: usize,
    pub multi_agent_mode: bool,
}

/// Template for legacy plan mode instructions.
#[derive(Template)]
#[template(path = "plan_mode/legacy.md")]
pub struct PlanModeLegacyPrompt {
    pub tools: ToolConfig,
    pub is_subagent: bool,
}

/// Template for sub-agent plan mode instructions.
#[derive(Template)]
#[template(path = "plan_mode/subagent.md")]
pub struct PlanModeSubagentPrompt {
    pub tools: ToolConfig,
    pub plan_file_info: String,
}

/// Template for plan mode reentry instructions.
#[derive(Template)]
#[template(path = "plan_mode/reentry.md")]
pub struct PlanModeReentryPrompt {
    pub tools: ToolConfig,
    pub plan_file_path: String,
}

// =============================================================================
// Tool Handler Messages (inline templates)
// =============================================================================

/// Message shown when entering plan mode successfully.
#[derive(Template)]
#[template(
    ext = "txt",
    source = r#"Entered plan mode. You should now focus on exploring the codebase and designing an implementation approach.


In plan mode, you should:
1. Thoroughly explore the codebase to understand existing patterns
2. Identify similar features and architectural approaches
3. Consider multiple approaches and their trade-offs
4. Use {{ tools.ask_user_question_tool() }} if you need to clarify the approach
5. Design a concrete implementation strategy
6. When ready, use {{ tools.exit_plan_mode_tool() }} to present your plan for approval

Remember: DO NOT write or edit any files yet. This is a read-only exploration and planning phase."#
)]
pub struct EnterPlanModeSuccessMessage {
    pub tools: ToolConfig,
}

/// Error message when plan file doesn't exist on exit.
#[derive(Template)]
#[template(
    ext = "txt",
    source = "No plan file found at {{ path_str }}. Please write your plan to this file before calling {{ tools.exit_plan_mode_tool() }}."
)]
pub struct ExitPlanModeNoFileError {
    pub tools: ToolConfig,
    pub path_str: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use askama::Template;

    #[test]
    fn test_template_with_file_edit() {
        let tools = ToolConfig::new(Some(EditToolType::FileEdit), ConfigShellToolType::Bash);
        let template = GeneralMainPrompt { tools };
        let rendered = template.render().unwrap();

        // Variable tool names should be substituted
        assert!(rendered.contains("edit_file"));
        assert!(rendered.contains("bash"));

        // Conditional apply_patch section should be absent
        assert!(!rendered.contains("## apply_patch"));
    }

    #[test]
    fn test_template_with_apply_patch() {
        let tools = ToolConfig::new(
            Some(EditToolType::ApplyPatchFreeform),
            ConfigShellToolType::ShellCommand,
        );
        let template = GeneralMainPrompt { tools };
        let rendered = template.render().unwrap();

        // apply_patch tool name should appear
        assert!(rendered.contains("apply_patch"));
        assert!(rendered.contains("shell_command"));

        // Conditional apply_patch section SHOULD be present
        assert!(rendered.contains("## apply_patch"));
    }

    #[test]
    fn test_template_no_edit_tool() {
        let tools = ToolConfig::new(None, ConfigShellToolType::Disabled);
        let template = GeneralMainPrompt { tools };
        let rendered = template.render().unwrap();

        // No edit tool - should have some indicator or be absent
        assert!(!rendered.contains("## apply_patch"));
    }

    #[test]
    fn test_template_shell_variants() {
        // Test Local/Default -> shell
        let tools = ToolConfig::new(Some(EditToolType::FileEdit), ConfigShellToolType::Local);
        let template = GeneralMainPrompt { tools };
        let rendered = template.render().unwrap();
        assert!(rendered.contains("shell"));

        // Test UnifiedExec -> exec_command
        let tools = ToolConfig::new(
            Some(EditToolType::FileEdit),
            ConfigShellToolType::UnifiedExec,
        );
        let template = GeneralMainPrompt { tools };
        let rendered = template.render().unwrap();
        assert!(rendered.contains("exec_command"));
    }

    #[test]
    fn test_fixed_tool_names() {
        let tools = ToolConfig::new(Some(EditToolType::FileEdit), ConfigShellToolType::Bash);
        let template = GeneralMainPrompt { tools };
        let rendered = template.render().unwrap();

        // Fixed tool names should be rendered from constants
        assert!(rendered.contains("todo_write"));
        assert!(rendered.contains("ask_user_question"));
        assert!(rendered.contains("task"));
        assert!(rendered.contains("read_file"));
        assert!(rendered.contains("glob"));
        assert!(rendered.contains("grep_files"));
    }
}
