use codex_protocol::config_types::CollaborationMode;

pub(super) fn builtin_collaboration_mode_presets() -> Vec<CollaborationMode> {
    vec![
        CollaborationMode::Plan,
        CollaborationMode::PairProgramming,
        CollaborationMode::Execute,
    ]
}
