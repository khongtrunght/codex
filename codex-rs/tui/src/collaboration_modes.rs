use codex_core::models_manager::manager::ModelsManager;
use codex_protocol::config_types::CollaborationMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModeKind {
    Plan,
    PairProgramming,
    Execute,
}

fn mode_kind(mode: &CollaborationMode) -> ModeKind {
    match mode {
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
