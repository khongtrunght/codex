use codex_core::models_manager::manager::ModelsManager;
use codex_protocol::config_types::CollaborationMode;
use ratatui::style::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModeKind {
    None,
    Plan,
    PairProgramming,
    Execute,
    // Custom,
}

/// Get the icon for a collaboration mode.
pub(crate) fn icon(mode: &CollaborationMode) -> &'static str {
    match mode {
        CollaborationMode::None => "",         // No icon for None mode
        CollaborationMode::Plan => "\u{23F8}", // ⏸ (pause - thoughtful planning)
        CollaborationMode::PairProgramming => "\u{21C4}", // ⇄ (bidirectional collaboration)
        CollaborationMode::Execute => "\u{23F5}", // ⏵ (play - autonomous execution)
    }
}

/// Get the display name for a collaboration mode.
pub(crate) fn display_name(mode: &CollaborationMode) -> &'static str {
    match mode {
        CollaborationMode::None => "None",
        CollaborationMode::Plan => "Plan",
        CollaborationMode::PairProgramming => "Pair Programming",
        CollaborationMode::Execute => "Execute",
    }
}

/// Get the color for a collaboration mode.
pub(crate) fn color(mode: &CollaborationMode) -> Color {
    match mode {
        CollaborationMode::None => Color::Reset, // Default terminal color
        CollaborationMode::Plan => Color::Cyan,
        CollaborationMode::PairProgramming => Color::Yellow,
        CollaborationMode::Execute => Color::Green,
    }
}

fn mode_kind(mode: &CollaborationMode) -> ModeKind {
    match mode {
        CollaborationMode::None => ModeKind::None,
        CollaborationMode::Plan => ModeKind::Plan,
        CollaborationMode::PairProgramming => ModeKind::PairProgramming,
        CollaborationMode::Execute => ModeKind::Execute,
    }
}

pub(crate) fn same_variant(a: &CollaborationMode, b: &CollaborationMode) -> bool {
    mode_kind(a) == mode_kind(b)
}

/// Cycle to the next collaboration mode preset in list order.
pub(crate) fn next_mode(
    models_manager: &ModelsManager,
    current: &CollaborationMode,
) -> Option<CollaborationMode> {
    let presets = models_manager.list_collaboration_modes();
    if presets.is_empty() {
        return None;
    }
    let current_kind = mode_kind(current);
    let next_index = presets
        .iter()
        .position(|preset| mode_kind(preset) == current_kind)
        .map_or(0, |idx| (idx + 1) % presets.len());
    presets.get(next_index).cloned()
}
