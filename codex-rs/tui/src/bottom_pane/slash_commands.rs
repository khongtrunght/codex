//! Shared helpers for filtering and matching built-in slash commands.
//!
//! The same sandbox- and feature-gating rules are used by both the composer
//! and the command popup. Centralizing them here keeps those call sites small
//! and ensures they stay in sync.
use crate::slash_command::SlashCommand;
use crate::slash_command::built_in_slash_commands;

/// Return the built-ins that should be visible/usable for the current input.
pub(crate) fn builtins_for_input(
    collaboration_modes_enabled: bool,
    connectors_enabled: bool,
    personality_command_enabled: bool,
    allow_elevate_sandbox: bool,
) -> Vec<(&'static str, SlashCommand)> {
    let _ = connectors_enabled;
    let _ = personality_command_enabled;
    built_in_slash_commands()
        .into_iter()
        .filter(|(_, cmd)| allow_elevate_sandbox || *cmd != SlashCommand::ElevateSandbox)
        .filter(|(_, cmd)| collaboration_modes_enabled || !matches!(*cmd, SlashCommand::Collab))
        .collect()
}
