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
pub enum DisplayVerbosity {
    /// Show truncated output (default: TOOL_CALL_MAX_LINES)
    #[default]
    Compact,
    /// Show expanded output (TOOL_CALL_VERBOSE_MAX_LINES)
    Verbose,
}

impl DisplayVerbosity {
    /// Returns true if verbose mode is enabled.
    #[inline]
    pub const fn is_verbose(self) -> bool {
        matches!(self, Self::Verbose)
    }

    /// Toggle between Compact and Verbose.
    #[inline]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Compact => Self::Verbose,
            Self::Verbose => Self::Compact,
        }
    }

    /// Return the maximum lines for tool call output based on verbosity.
    #[inline]
    pub const fn max_output_lines(self) -> usize {
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
pub struct RenderContext {
    /// Available width in terminal columns.
    pub width: u16,
    /// Verbosity level for expandable content.
    pub verbosity: DisplayVerbosity,
}

impl RenderContext {
    /// Create a new render context with compact verbosity.
    #[inline]
    #[allow(dead_code)] // Constructor for default verbosity, kept for API completeness
    pub const fn new(width: u16) -> Self {
        Self {
            width,
            verbosity: DisplayVerbosity::Compact,
        }
    }

    /// Create a new render context with specified verbosity.
    #[inline]
    pub const fn with_verbosity(width: u16, verbosity: DisplayVerbosity) -> Self {
        Self { width, verbosity }
    }

    /// Return true if verbose mode is enabled.
    #[inline]
    pub const fn is_verbose(self) -> bool {
        self.verbosity.is_verbose()
    }
}
