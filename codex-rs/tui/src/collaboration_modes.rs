use codex_core::models_manager::manager::ModelsManager;
use codex_protocol::config_types::CollaborationMode;
use ratatui::style::Color;

/// Get the icon for a collaboration mode.
pub(crate) fn icon(mode: CollaborationMode) -> &'static str {
    match mode {
        CollaborationMode::Code => "\u{23F5}", // ⏵ (play - active coding)
        CollaborationMode::Plan => "\u{23F8}", // ⏸ (pause - thoughtful planning)
    }
}

/// Get the display name for a collaboration mode.
pub(crate) fn display_name(mode: CollaborationMode) -> &'static str {
    match mode {
        CollaborationMode::Code => "Code",
        CollaborationMode::Plan => "Plan",
    }
}

/// Get the color for a collaboration mode.
pub(crate) fn color(mode: CollaborationMode) -> Color {
    match mode {
        CollaborationMode::Code => Color::Green, // Green for active coding
        CollaborationMode::Plan => Color::Cyan,  // Cyan for planning
    }
}

pub(crate) fn same_variant(a: CollaborationMode, b: CollaborationMode) -> bool {
    a == b
}

/// Cycle to the next collaboration mode preset in list order.
/// Only cycles between Code (None) and Plan modes.
pub(crate) fn next_mode(
    models_manager: &ModelsManager,
    current: CollaborationMode,
) -> Option<CollaborationMode> {
    let presets = models_manager.list_collaboration_modes();
    if presets.is_empty() {
        return None;
    }

    let next_index = presets
        .iter()
        .position(|preset| *preset == current)
        .map_or(0, |idx| (idx + 1) % presets.len());
    presets.get(next_index).cloned()
}
