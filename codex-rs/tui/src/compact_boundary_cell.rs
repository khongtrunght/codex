use crate::history_cell::HistoryCell;
use crate::verbosity::DisplayVerbosity;
use crate::verbosity::RenderContext;
use codex_core::protocol::RestoredFileInfo;
use ratatui::prelude::*;
use ratatui::style::Stylize;
use unicode_width::UnicodeWidthStr;

/// Cell displaying a compaction boundary with restored files.
/// ```text
/// ═══════════════ Conversation compacted · ctrl+o for history ═══════════════
/// L  Referenced file thoughts/shared/plans/2026-01-12-auto-compact-...md
/// L  Read codex-rs/core/src/read_file_state.rs (337 lines)
/// ```
#[derive(Debug)]
pub(crate) struct CompactBoundaryCell {
    restored_files: Vec<RestoredFileInfo>,
    /// The compact summary text for display when user presses ctrl+o.
    summary: Option<String>,
}

impl CompactBoundaryCell {
    pub(crate) fn new(restored_files: Vec<RestoredFileInfo>, summary: Option<String>) -> Self {
        Self {
            restored_files,
            summary,
        }
    }
}

impl HistoryCell for CompactBoundaryCell {
    fn display_lines(&self, ctx: RenderContext) -> Vec<Line<'static>> {
        self.render_boundary(ctx.width, ctx.verbosity == DisplayVerbosity::Verbose)
    }
}

impl CompactBoundaryCell {
    fn render_boundary(&self, width: u16, verbose: bool) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        // Divider line with centered title
        let title = if verbose {
            " Conversation compacted (showing summary) "
        } else {
            " Conversation compacted · ctrl+o for history "
        };
        let title_width = title.width();
        let available_for_dividers = (width as usize).saturating_sub(title_width);
        let left_divider_len = available_for_dividers / 2;
        let right_divider_len = available_for_dividers.saturating_sub(left_divider_len);

        let left_divider = "═".repeat(left_divider_len);
        let right_divider = "═".repeat(right_divider_len);
        let divider_line = format!("{left_divider}{title}{right_divider}");
        lines.push(Line::from(divider_line).dim());

        // Show summary when in verbose mode
        if verbose && let Some(summary) = self.summary.as_ref() {
            lines.push(Line::from("").dim());
            for line in summary.lines() {
                lines.push(Line::from(line.to_string()).dim());
            }
            lines.push(Line::from("").dim());
        }

        // Restored files
        for file in &self.restored_files {
            let path = &file.path;
            let line = if let Some(num_lines) = file.num_lines {
                format!("L  Read {path} ({num_lines} lines)")
            } else {
                format!("L  Referenced file {path}")
            };
            lines.push(Line::from(line).dim());
        }

        lines
    }
}
