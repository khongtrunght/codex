//! Display verbosity types for controlling detail level in history cells.
//!
//! This module provides the [`DisplayVerbosity`] enum and [`RenderContext`] struct
//! that control how much detail is shown when rendering expandable content in the TUI.

use crate::exec_cell::TOOL_CALL_MAX_LINES;
use crate::exec_cell::TOOL_CALL_VERBOSE_MAX_LINES;

/// Controls how much detail is shown in expandable history cells.
///
/// By default, cells show a compact view with truncated output. Verbose mode
/// expands cells to show full content (up to a higher limit), which is useful
/// for debugging or reviewing complete tool output.
///
/// Toggle with `Ctrl+O` at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DisplayVerbosity {
    /// Show truncated output (default: TOOL_CALL_MAX_LINES)
    #[default]
    Compact,
    /// Show expanded output (TOOL_CALL_VERBOSE_MAX_LINES)
    Verbose,
}

impl DisplayVerbosity {
    /// Toggle between Compact and Verbose.
    #[inline]
    pub(crate) const fn toggle(self) -> Self {
        match self {
            Self::Compact => Self::Verbose,
            Self::Verbose => Self::Compact,
        }
    }

    /// Return the maximum lines for tool call output based on verbosity.
    #[inline]
    pub(crate) const fn max_output_lines(self) -> usize {
        match self {
            Self::Compact => TOOL_CALL_MAX_LINES,
            Self::Verbose => TOOL_CALL_VERBOSE_MAX_LINES,
        }
    }
}

/// Rendering context passed to history cell display methods.
///
/// This struct bundles all parameters needed for rendering, allowing methods
/// to remain stable as new rendering options are added. Currently includes
/// width and verbosity; future additions might include theme, animation state,
/// or selection highlighting.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RenderContext {
    /// Available width in terminal columns.
    pub(crate) width: u16,
    /// Verbosity level for expandable content.
    pub(crate) verbosity: DisplayVerbosity,
}

impl RenderContext {
    /// Create a new render context with compact verbosity.
    #[inline]
    pub(crate) const fn new(width: u16) -> Self {
        Self {
            width,
            verbosity: DisplayVerbosity::Compact,
        }
    }

    /// Create a new render context with specified verbosity.
    #[inline]
    pub(crate) const fn with_verbosity(width: u16, verbosity: DisplayVerbosity) -> Self {
        Self { width, verbosity }
    }
}
