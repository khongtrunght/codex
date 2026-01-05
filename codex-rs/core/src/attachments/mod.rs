//! Generic attachments system for injecting contextual prompts.
//!
//! This module provides an extensible way to collect and inject attachments
//! into the conversation. Each attachment type can be collected independently
//! and converted to ResponseItems for prompt injection.
//!
//! ## Adding a new attachment type:
//!
//! 1. Add a variant to `AttachmentData` enum in `protocol/models.rs`
//! 2. Implement conversion in `attachment_data_to_messages()` in `types.rs`
//! 3. Create a collector function in `collectors.rs`
//! 4. Register the collector in `default_registry()`

mod collectors;
mod registry;
mod types;

pub use collectors::collect_plan_mode;
pub use collectors::collect_plan_mode_exit;
pub use registry::AttachmentRegistry;
pub use types::expand_attachments;
pub use types::ToolsConfig;

/// Create a registry with default collectors.
///
/// This is the standard registry used by sessions.
pub fn default_registry() -> AttachmentRegistry {
    let mut registry = AttachmentRegistry::new();
    registry.register(collect_plan_mode);
    registry.register(collect_plan_mode_exit);
    // Future collectors can be added here:
    // registry.register(collect_todo);
    // registry.register(collect_diagnostics);
    registry
}
