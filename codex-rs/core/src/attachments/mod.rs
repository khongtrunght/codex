//! Generic attachments system for injecting contextual prompts.
//!
//! This module provides an extensible way to collect and inject attachments
//! into the conversation. Each attachment type can be collected independently
//! and converted to ResponseItems for prompt injection.
//!
//! ## Adding a new attachment type:
//!
//! 1. Add a variant to `Attachment` enum in `types.rs`
//! 2. Implement conversion in `Attachment::into_response_items()`
//! 3. Create a collector function in `collectors.rs`
//! 4. Register the collector in `default_registry()`

mod collectors;
mod registry;
mod types;

pub use collectors::collect_plan_mode;
pub use registry::AttachmentRegistry;
pub use types::ToolsConfig;

// Re-export Attachment for external consumers who may want to create attachments directly
#[allow(unused_imports)]
pub use types::Attachment;

/// Create a registry with default collectors.
///
/// This is the standard registry used by sessions.
pub fn default_registry() -> AttachmentRegistry {
    let mut registry = AttachmentRegistry::new();
    registry.register(collect_plan_mode);
    // Future collectors can be added here:
    // registry.register(collect_todo);
    // registry.register(collect_diagnostics);
    registry
}
