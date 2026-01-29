use codex_protocol::config_types::CollaborationMode;

/// Returns the list of available collaboration mode presets.
/// Only Code (None) and Plan modes are supported.
pub(super) fn builtin_collaboration_mode_presets() -> Vec<CollaborationMode> {
    vec![CollaborationMode::Code, CollaborationMode::Plan]
}
