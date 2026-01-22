//! Attachment collection and expansion system.
//!
//! Attachments are contextual markers that get collected during turns
//! and expanded into actual messages just before API calls.
//! This follows the GhostSnapshot pattern: stored in history but transformed before use.

mod collectors;
mod types;

pub(crate) use collectors::collect_plan_mode;
pub(crate) use collectors::collect_plan_mode_exit;
pub(crate) use types::expand_attachment;
