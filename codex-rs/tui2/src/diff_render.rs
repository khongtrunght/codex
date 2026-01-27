use diffy::Hunk;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line as RtLine;
use ratatui::text::Span as RtSpan;
use ratatui::widgets::Paragraph;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

use crate::exec_command::relativize_to_home;
use crate::render::Insets;
use crate::render::line_utils::prefix_lines;
use crate::render::line_utils::push_owned_lines;
use crate::render::renderable::ColumnRenderable;
use crate::render::renderable::InsetRenderable;
use crate::render::renderable::Renderable;
use crate::render::syntax_highlight::highlight_code_to_lines;
use crate::wrapping::RtOptions;
use crate::wrapping::word_wrap_line;
use codex_core::git_info::get_git_repo_root;
use codex_core::protocol::FileChange;

// Internal representation for diff line rendering
#[derive(Copy, Clone)]
enum DiffLineType {
    Insert,
    Delete,
    Context,
}

pub struct DiffSummary {
    changes: HashMap<PathBuf, FileChange>,
    cwd: PathBuf,
}

impl DiffSummary {
    pub fn new(changes: HashMap<PathBuf, FileChange>, cwd: PathBuf) -> Self {
        Self { changes, cwd }
    }
}

impl Renderable for FileChange {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let mut lines = vec![];
        render_change(self, &mut lines, area.width as usize);
        Paragraph::new(lines).render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        let mut lines = vec![];
        render_change(self, &mut lines, width as usize);
        lines.len() as u16
    }
}

/// Renderable FileChange with path context for language detection.
struct FileChangeWithPath {
    change: FileChange,
    path: PathBuf,
}

impl Renderable for FileChangeWithPath {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let mut lines = vec![];
        render_change_with_path(
            &self.change,
            &mut lines,
            area.width as usize,
            Some(self.path.as_path()),
        );
        Paragraph::new(lines).render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        let mut lines = vec![];
        render_change_with_path(
            &self.change,
            &mut lines,
            width as usize,
            Some(self.path.as_path()),
        );
        lines.len() as u16
    }
}

impl From<DiffSummary> for Box<dyn Renderable> {
    fn from(val: DiffSummary) -> Self {
        let mut rows: Vec<Box<dyn Renderable>> = vec![];

        for (i, row) in collect_rows(&val.changes).into_iter().enumerate() {
            if i > 0 {
                rows.push(Box::new(RtLine::from("")));
            }
            let Row {
                path,
                move_path,
                added,
                removed,
                change,
            } = row;
            let mut path_line = RtLine::from(display_path_for(&path, &val.cwd));
            path_line.push_span(" ");
            path_line.extend(render_line_count_summary(added, removed));
            rows.push(Box::new(path_line));
            rows.push(Box::new(RtLine::from("")));
            let highlight_path = move_path.clone().unwrap_or_else(|| path.clone());
            rows.push(Box::new(InsetRenderable::new(
                Box::new(FileChangeWithPath {
                    change,
                    path: highlight_path,
                }) as Box<dyn Renderable>,
                Insets::tlbr(0, 2, 0, 0),
            )));
        }

        Box::new(ColumnRenderable::with(rows))
    }
}

pub(crate) fn create_diff_summary(
    changes: &HashMap<PathBuf, FileChange>,
    cwd: &Path,
    wrap_cols: usize,
) -> Vec<RtLine<'static>> {
    let rows = collect_rows(changes);
    render_changes_block(rows, wrap_cols, cwd)
}

// Shared row for per-file presentation
#[derive(Clone)]
struct Row {
    #[allow(dead_code)]
    path: PathBuf,
    move_path: Option<PathBuf>,
    added: usize,
    removed: usize,
    change: FileChange,
}

fn collect_rows(changes: &HashMap<PathBuf, FileChange>) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for (path, change) in changes.iter() {
        let (added, removed) = match change {
            FileChange::Add { content } => (content.lines().count(), 0),
            FileChange::Delete { content } => (0, content.lines().count()),
            FileChange::Update { unified_diff, .. } => calculate_add_remove_from_diff(unified_diff),
        };
        let move_path = match change {
            FileChange::Update {
                move_path: Some(new),
                ..
            } => Some(new.clone()),
            _ => None,
        };
        rows.push(Row {
            path: path.clone(),
            move_path,
            added,
            removed,
            change: change.clone(),
        });
    }
    rows.sort_by_key(|r| r.path.clone());
    rows
}

fn render_line_count_summary(added: usize, removed: usize) -> Vec<RtSpan<'static>> {
    let mut spans = Vec::new();
    spans.push("(".into());
    spans.push(format!("+{added}").green());
    spans.push(" ".into());
    spans.push(format!("-{removed}").red());
    spans.push(")".into());
    spans
}

fn render_changes_block(rows: Vec<Row>, wrap_cols: usize, cwd: &Path) -> Vec<RtLine<'static>> {
    let mut out: Vec<RtLine<'static>> = Vec::new();

    let render_path = |row: &Row| -> Vec<RtSpan<'static>> {
        let mut spans = Vec::new();
        spans.push(display_path_for(&row.path, cwd).into());
        if let Some(move_path) = &row.move_path {
            spans.push(format!(" → {}", display_path_for(move_path, cwd)).into());
        }
        spans
    };

    // Header
    let total_added: usize = rows.iter().map(|r| r.added).sum();
    let total_removed: usize = rows.iter().map(|r| r.removed).sum();
    let file_count = rows.len();
    let noun = if file_count == 1 { "file" } else { "files" };
    let mut header_spans: Vec<RtSpan<'static>> = vec!["• ".dim()];
    if let [row] = &rows[..] {
        let verb = match &row.change {
            FileChange::Add { .. } => "Added",
            FileChange::Delete { .. } => "Deleted",
            _ => "Edited",
        };
        header_spans.push(verb.bold());
        header_spans.push(" ".into());
        header_spans.extend(render_path(row));
        header_spans.push(" ".into());
        header_spans.extend(render_line_count_summary(row.added, row.removed));
    } else {
        header_spans.push("Edited".bold());
        header_spans.push(format!(" {file_count} {noun} ").into());
        header_spans.extend(render_line_count_summary(total_added, total_removed));
    }
    out.push(RtLine::from(header_spans));

    for (idx, r) in rows.into_iter().enumerate() {
        // Insert a blank separator between file chunks (except before the first)
        if idx > 0 {
            out.push("".into());
        }
        // File header line (skip when single-file header already shows the name)
        let skip_file_header = file_count == 1;
        if !skip_file_header {
            let mut header: Vec<RtSpan<'static>> = Vec::new();
            header.push("  └ ".dim());
            header.extend(render_path(&r));
            header.push(" ".into());
            header.extend(render_line_count_summary(r.added, r.removed));
            out.push(RtLine::from(header));
        }

        let mut lines = vec![];
        render_change_with_path(&r.change, &mut lines, wrap_cols - 4, Some(&r.path));
        out.extend(prefix_lines(lines, "    ".into(), "    ".into()));
    }

    out
}

fn render_change(change: &FileChange, out: &mut Vec<RtLine<'static>>, width: usize) {
    render_change_with_path(change, out, width, None);
}

/// Render a file change with optional path for language detection.
fn render_change_with_path(
    change: &FileChange,
    out: &mut Vec<RtLine<'static>>,
    width: usize,
    path: Option<&Path>,
) {
    match change {
        FileChange::Add { content } => {
            let line_number_width = line_number_width(content.lines().count());
            let lang = path.and_then(detect_language_from_path);

            if let Some(lang) = lang {
                // Use syntax highlighting
                let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
                let highlighted_lines = highlight_lines(&lines, Some(lang));

                for (i, highlighted_line) in highlighted_lines.into_iter().enumerate() {
                    out.extend(push_wrapped_diff_line_highlighted(
                        i + 1,
                        DiffLineType::Insert,
                        highlighted_line,
                        width,
                        line_number_width,
                    ));
                }
            } else {
                // Fall back to plain rendering
                for (i, raw) in content.lines().enumerate() {
                    out.extend(push_wrapped_diff_line(
                        i + 1,
                        DiffLineType::Insert,
                        raw,
                        width,
                        line_number_width,
                    ));
                }
            }
        }
        FileChange::Delete { content } => {
            let line_number_width = line_number_width(content.lines().count());
            let lang = path.and_then(detect_language_from_path);

            if let Some(lang) = lang {
                // Use syntax highlighting
                let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
                let highlighted_lines = highlight_lines(&lines, Some(lang));

                for (i, highlighted_line) in highlighted_lines.into_iter().enumerate() {
                    out.extend(push_wrapped_diff_line_highlighted(
                        i + 1,
                        DiffLineType::Delete,
                        highlighted_line,
                        width,
                        line_number_width,
                    ));
                }
            } else {
                // Fall back to plain rendering
                for (i, raw) in content.lines().enumerate() {
                    out.extend(push_wrapped_diff_line(
                        i + 1,
                        DiffLineType::Delete,
                        raw,
                        width,
                        line_number_width,
                    ));
                }
            }
        }
        FileChange::Update { unified_diff, .. } => {
            if let Ok(patch) = diffy::Patch::from_str(unified_diff) {
                let lang = path.and_then(detect_language_from_path);

                // Calculate max line number for gutter width
                let mut max_line_number = 0;
                for h in patch.hunks() {
                    let mut old_ln = h.old_range().start();
                    let mut new_ln = h.new_range().start();
                    for l in h.lines() {
                        match l {
                            diffy::Line::Insert(_) => {
                                max_line_number = max_line_number.max(new_ln);
                                new_ln += 1;
                            }
                            diffy::Line::Delete(_) => {
                                max_line_number = max_line_number.max(old_ln);
                                old_ln += 1;
                            }
                            diffy::Line::Context(_) => {
                                max_line_number = max_line_number.max(new_ln);
                                old_ln += 1;
                                new_ln += 1;
                            }
                        }
                    }
                }
                let line_number_width = line_number_width(max_line_number);

                // Render each hunk
                let mut is_first_hunk = true;
                for h in patch.hunks() {
                    if !is_first_hunk {
                        let spacer = format!("{:width$} ", "", width = line_number_width.max(1));
                        let spacer_span = RtSpan::styled(spacer, style_gutter());
                        out.push(RtLine::from(vec![spacer_span, "⋮".dim()]));
                    }
                    is_first_hunk = false;

                    if let Some(lang) = lang {
                        // Use syntax highlighting per-hunk (color-diff pattern)
                        render_hunk_with_syntax(h, lang, width, line_number_width, out);
                    } else {
                        // Fall back to plain rendering
                        render_hunk_plain(h, width, line_number_width, out);
                    }
                }
            }
        }
    }
}

/// Render a hunk with syntax highlighting (following color-diff pattern).
fn render_hunk_with_syntax(
    hunk: &diffy::Hunk<str>,
    lang: &str,
    width: usize,
    line_number_width: usize,
    out: &mut Vec<RtLine<'static>>,
) {
    // Reconstruct old and new content from hunk (color-diff pattern)
    let (old_content, new_content) = reconstruct_hunk_content(hunk);

    // Highlight both versions
    let old_highlighted = highlight_lines(&old_content, Some(lang));
    let new_highlighted = highlight_lines(&new_content, Some(lang));

    // Iterate through hunk lines with separate indices
    let mut old_idx = 0;
    let mut new_idx = 0;
    let mut old_ln = hunk.old_range().start();
    let mut new_ln = hunk.new_range().start();

    for line in hunk.lines() {
        match line {
            diffy::Line::Context(_) => {
                let highlighted_line = new_highlighted
                    .get(new_idx)
                    .cloned()
                    .unwrap_or_else(|| RtLine::from(""));
                out.extend(push_wrapped_diff_line_highlighted(
                    new_ln,
                    DiffLineType::Context,
                    highlighted_line,
                    width,
                    line_number_width,
                ));
                old_idx += 1;
                new_idx += 1;
                old_ln += 1;
                new_ln += 1;
            }
            diffy::Line::Delete(_) => {
                let highlighted_line = old_highlighted
                    .get(old_idx)
                    .cloned()
                    .unwrap_or_else(|| RtLine::from(""));
                out.extend(push_wrapped_diff_line_highlighted(
                    old_ln,
                    DiffLineType::Delete,
                    highlighted_line,
                    width,
                    line_number_width,
                ));
                old_idx += 1;
                old_ln += 1;
            }
            diffy::Line::Insert(_) => {
                let highlighted_line = new_highlighted
                    .get(new_idx)
                    .cloned()
                    .unwrap_or_else(|| RtLine::from(""));
                out.extend(push_wrapped_diff_line_highlighted(
                    new_ln,
                    DiffLineType::Insert,
                    highlighted_line,
                    width,
                    line_number_width,
                ));
                new_idx += 1;
                new_ln += 1;
            }
        }
    }
}

/// Render a hunk without syntax highlighting (plain text fallback).
fn render_hunk_plain(
    hunk: &diffy::Hunk<str>,
    width: usize,
    line_number_width: usize,
    out: &mut Vec<RtLine<'static>>,
) {
    let mut old_ln = hunk.old_range().start();
    let mut new_ln = hunk.new_range().start();

    for l in hunk.lines() {
        match l {
            diffy::Line::Insert(text) => {
                let s = text.trim_end_matches('\n');
                out.extend(push_wrapped_diff_line(
                    new_ln,
                    DiffLineType::Insert,
                    s,
                    width,
                    line_number_width,
                ));
                new_ln += 1;
            }
            diffy::Line::Delete(text) => {
                let s = text.trim_end_matches('\n');
                out.extend(push_wrapped_diff_line(
                    old_ln,
                    DiffLineType::Delete,
                    s,
                    width,
                    line_number_width,
                ));
                old_ln += 1;
            }
            diffy::Line::Context(text) => {
                let s = text.trim_end_matches('\n');
                out.extend(push_wrapped_diff_line(
                    new_ln,
                    DiffLineType::Context,
                    s,
                    width,
                    line_number_width,
                ));
                old_ln += 1;
                new_ln += 1;
            }
        }
    }
}

/// Format a path for display relative to the current working directory when
/// possible, keeping output stable in jj/no-`.git` workspaces (e.g. image
/// tool calls should show `example.png` instead of an absolute path).
pub(crate) fn display_path_for(path: &Path, cwd: &Path) -> String {
    if path.is_relative() {
        return path.display().to_string();
    }

    if let Ok(stripped) = path.strip_prefix(cwd) {
        return stripped.display().to_string();
    }

    // Prefer a stable, user-local relative path when the file is under the current working
    // directory. This keeps output deterministic in jj-only repos (no `.git`) and matches user
    // expectations for "files in this project".
    if let Some(rel) = pathdiff::diff_paths(path, cwd)
        && !rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return rel.display().to_string();
    }

    let path_in_same_repo = match (get_git_repo_root(cwd), get_git_repo_root(path)) {
        (Some(cwd_repo), Some(path_repo)) => cwd_repo == path_repo,
        _ => false,
    };
    let chosen = if path_in_same_repo {
        pathdiff::diff_paths(path, cwd).unwrap_or_else(|| path.to_path_buf())
    } else {
        relativize_to_home(path)
            .map(|p| PathBuf::from_iter([Path::new("~"), p.as_path()]))
            .unwrap_or_else(|| path.to_path_buf())
    };
    chosen.display().to_string()
}

fn calculate_add_remove_from_diff(diff: &str) -> (usize, usize) {
    if let Ok(patch) = diffy::Patch::from_str(diff) {
        patch
            .hunks()
            .iter()
            .flat_map(Hunk::lines)
            .fold((0, 0), |(a, d), l| match l {
                diffy::Line::Insert(_) => (a + 1, d),
                diffy::Line::Delete(_) => (a, d + 1),
                diffy::Line::Context(_) => (a, d),
            })
    } else {
        // For unparsable diffs, return 0 for both counts.
        (0, 0)
    }
}

fn push_wrapped_diff_line(
    line_number: usize,
    kind: DiffLineType,
    text: &str,
    width: usize,
    line_number_width: usize,
) -> Vec<RtLine<'static>> {
    wrap_diff_line(
        line_number,
        kind,
        RtLine::from(text.to_string()),
        width,
        line_number_width,
    )
}

/// Push a wrapped diff line with syntax highlighting.
/// This version takes a pre-highlighted line and applies diff colors on top.
fn push_wrapped_diff_line_highlighted(
    line_number: usize,
    kind: DiffLineType,
    highlighted_line: RtLine<'static>,
    width: usize,
    line_number_width: usize,
) -> Vec<RtLine<'static>> {
    wrap_diff_line(
        line_number,
        kind,
        highlighted_line,
        width,
        line_number_width,
    )
}

fn line_number_width(max_line_number: usize) -> usize {
    if max_line_number == 0 {
        1
    } else {
        max_line_number.to_string().len()
    }
}

fn style_gutter() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

fn style_context() -> Style {
    Style::default()
}

fn style_add() -> Style {
    Style::default().bg(Color::Rgb(32, 56, 43)) // Tokyo Night dark green background
}

fn style_del() -> Style {
    Style::default().bg(Color::Rgb(56, 32, 43)) // Tokyo Night dark red background
}

/// Detect language from file path extension (for syntax highlighting).
/// Returns language name compatible with syntax_highlight module.
fn detect_language_from_path(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?;

    match ext.to_lowercase().as_str() {
        "rs" => Some("rust"),
        "py" | "pyw" => Some("python"),
        "js" | "mjs" | "cjs" => Some("javascript"),
        "ts" | "mts" | "cts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "go" => Some("go"),
        "json" => Some("json"),
        "sh" | "bash" | "zsh" => Some("bash"),
        _ => None,
    }
}

/// Reconstruct old and new content from a diffy hunk.
/// This follows the exact same logic as color-diff's reconstruct_content.
fn reconstruct_hunk_content<'a>(hunk: &diffy::Hunk<'a, str>) -> (Vec<String>, Vec<String>) {
    let mut old_lines = Vec::new();
    let mut new_lines = Vec::new();

    for line in hunk.lines() {
        match line {
            diffy::Line::Context(text) => {
                let content = text.trim_end_matches('\n');
                old_lines.push(content.to_string());
                new_lines.push(content.to_string());
            }
            diffy::Line::Delete(text) => {
                let content = text.trim_end_matches('\n');
                old_lines.push(content.to_string());
            }
            diffy::Line::Insert(text) => {
                let content = text.trim_end_matches('\n');
                new_lines.push(content.to_string());
            }
        }
    }

    (old_lines, new_lines)
}

/// Highlight lines of code using syntax highlighting.
/// Returns highlighted lines, or plain text lines if language is unsupported.
fn highlight_lines(content: &[String], lang: Option<&str>) -> Vec<RtLine<'static>> {
    if content.is_empty() {
        return vec![RtLine::from("")];
    }

    let Some(lang) = lang else {
        return content.iter().map(|s| RtLine::from(s.clone())).collect();
    };

    let joined = content.join("\n");
    highlight_code_to_lines(&joined, lang)
}

/// Apply diff background color on top of a line, preserving existing styles.
fn apply_diff_background(line: RtLine<'static>, diff_type: DiffLineType) -> RtLine<'static> {
    let bg_color = match diff_type {
        DiffLineType::Insert => Some(Color::Rgb(32, 56, 43)), // Tokyo Night dark green
        DiffLineType::Delete => Some(Color::Rgb(56, 32, 43)), // Tokyo Night dark red
        DiffLineType::Context => None,
    };

    let Some(bg_color) = bg_color else {
        return line;
    };

    let base_style = line.style;
    let spans: Vec<RtSpan<'static>> = line
        .spans
        .into_iter()
        .map(|mut span| {
            span.style = span.style.bg(bg_color);
            span
        })
        .collect();

    RtLine::from(spans).style(base_style)
}

fn wrap_diff_line(
    line_number: usize,
    kind: DiffLineType,
    line: RtLine<'static>,
    width: usize,
    line_number_width: usize,
) -> Vec<RtLine<'static>> {
    let gutter_width = line_number_width.max(1);
    let gutter = format!("{line_number:>gutter_width$} ");

    let line_style = match kind {
        DiffLineType::Insert => style_add(),
        DiffLineType::Delete => style_del(),
        DiffLineType::Context => style_context(),
    };
    let sign_char = match kind {
        DiffLineType::Insert => '+',
        DiffLineType::Delete => '-',
        DiffLineType::Context => ' ',
    };

    let colored_line = apply_diff_background(line, kind);
    let base_style = colored_line.style;
    let mut spans: Vec<RtSpan<'static>> = Vec::with_capacity(colored_line.spans.len() + 1);
    spans.push(RtSpan::styled(sign_char.to_string(), line_style));
    spans.extend(colored_line.spans);
    let line_with_sign = RtLine::from(spans).style(base_style);

    let initial_indent = RtLine::from(vec![RtSpan::styled(gutter, style_gutter())]);
    let subsequent_indent = match kind {
        DiffLineType::Context => RtLine::from(vec![RtSpan::styled(
            format!("{:gutter_width$}  ", ""),
            style_gutter(),
        )]),
        DiffLineType::Insert | DiffLineType::Delete => RtLine::from(vec![
            RtSpan::styled(format!("{:gutter_width$} ", ""), style_gutter()),
            RtSpan::styled(" ".to_string(), line_style),
        ]),
    };

    let opts = RtOptions::new(width.max(1))
        .initial_indent(initial_indent)
        .subsequent_indent(subsequent_indent);
    let wrapped = word_wrap_line(&line_with_sign, opts);

    let mut out = Vec::new();
    push_owned_lines(&wrapped, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use pretty_assertions::assert_eq;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::text::Text;
    use ratatui::widgets::Paragraph;
    use ratatui::widgets::WidgetRef;
    use ratatui::widgets::Wrap;
    fn diff_summary_for_tests(changes: &HashMap<PathBuf, FileChange>) -> Vec<RtLine<'static>> {
        create_diff_summary(changes, &PathBuf::from("/"), 80)
    }

    fn snapshot_lines(name: &str, lines: Vec<RtLine<'static>>, width: u16, height: u16) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|f| {
                Paragraph::new(Text::from(lines))
                    .wrap(Wrap { trim: false })
                    .render_ref(f.area(), f.buffer_mut())
            })
            .expect("draw");
        assert_snapshot!(name, terminal.backend());
    }

    fn snapshot_lines_text(name: &str, lines: &[RtLine<'static>]) {
        // Convert Lines to plain text rows and trim trailing spaces so it's
        // easier to validate indentation visually in snapshots.
        let text = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .map(|s| s.trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert_snapshot!(name, text);
    }

    #[test]
    fn display_path_prefers_cwd_without_git_repo() {
        let cwd = if cfg!(windows) {
            PathBuf::from(r"C:\workspace\codex")
        } else {
            PathBuf::from("/workspace/codex")
        };
        let path = cwd.join("tui").join("example.png");

        let rendered = display_path_for(&path, &cwd);

        assert_eq!(
            rendered,
            PathBuf::from("tui")
                .join("example.png")
                .display()
                .to_string()
        );
    }

    #[test]
    fn ui_snapshot_wrap_behavior_insert() {
        // Narrow width to force wrapping within our diff line rendering
        let long_line = "this is a very long line that should wrap across multiple terminal columns and continue";

        // Call the wrapping function directly so we can precisely control the width
        let lines =
            push_wrapped_diff_line(1, DiffLineType::Insert, long_line, 80, line_number_width(1));

        // Render into a small terminal to capture the visual layout
        snapshot_lines("wrap_behavior_insert", lines, 90, 8);
    }

    #[test]
    fn ui_snapshot_apply_update_block() {
        let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
        let original = "line one\nline two\nline three\n";
        let modified = "line one\nline two changed\nline three\n";
        let patch = diffy::create_patch(original, modified).to_string();

        changes.insert(
            PathBuf::from("example.txt"),
            FileChange::Update {
                unified_diff: patch,
                move_path: None,
            },
        );

        let lines = diff_summary_for_tests(&changes);

        snapshot_lines("apply_update_block", lines, 80, 12);
    }

    #[test]
    fn ui_snapshot_apply_update_with_rename_block() {
        let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
        let original = "A\nB\nC\n";
        let modified = "A\nB changed\nC\n";
        let patch = diffy::create_patch(original, modified).to_string();

        changes.insert(
            PathBuf::from("old_name.rs"),
            FileChange::Update {
                unified_diff: patch,
                move_path: Some(PathBuf::from("new_name.rs")),
            },
        );

        let lines = diff_summary_for_tests(&changes);

        snapshot_lines("apply_update_with_rename_block", lines, 80, 12);
    }

    #[test]
    fn ui_snapshot_apply_multiple_files_block() {
        // Two files: one update and one add, to exercise combined header and per-file rows
        let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();

        // File a.txt: single-line replacement (one delete, one insert)
        let patch_a = diffy::create_patch("one\n", "one changed\n").to_string();
        changes.insert(
            PathBuf::from("a.txt"),
            FileChange::Update {
                unified_diff: patch_a,
                move_path: None,
            },
        );

        // File b.txt: newly added with one line
        changes.insert(
            PathBuf::from("b.txt"),
            FileChange::Add {
                content: "new\n".to_string(),
            },
        );

        let lines = diff_summary_for_tests(&changes);

        snapshot_lines("apply_multiple_files_block", lines, 80, 14);
    }

    #[test]
    fn ui_snapshot_apply_add_block() {
        let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
        changes.insert(
            PathBuf::from("new_file.txt"),
            FileChange::Add {
                content: "alpha\nbeta\n".to_string(),
            },
        );

        let lines = diff_summary_for_tests(&changes);

        snapshot_lines("apply_add_block", lines, 80, 10);
    }

    #[test]
    fn ui_snapshot_apply_delete_block() {
        let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
        changes.insert(
            PathBuf::from("tmp_delete_example.txt"),
            FileChange::Delete {
                content: "first\nsecond\nthird\n".to_string(),
            },
        );

        let lines = diff_summary_for_tests(&changes);
        snapshot_lines("apply_delete_block", lines, 80, 12);
    }

    #[test]
    fn ui_snapshot_apply_update_block_wraps_long_lines() {
        // Create a patch with a long modified line to force wrapping
        let original = "line 1\nshort\nline 3\n";
        let modified = "line 1\nshort this_is_a_very_long_modified_line_that_should_wrap_across_multiple_terminal_columns_and_continue_even_further_beyond_eighty_columns_to_force_multiple_wraps\nline 3\n";
        let patch = diffy::create_patch(original, modified).to_string();

        let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
        changes.insert(
            PathBuf::from("long_example.txt"),
            FileChange::Update {
                unified_diff: patch,
                move_path: None,
            },
        );

        let lines = create_diff_summary(&changes, &PathBuf::from("/"), 72);

        // Render with backend width wider than wrap width to avoid Paragraph auto-wrap.
        snapshot_lines("apply_update_block_wraps_long_lines", lines, 80, 12);
    }

    #[test]
    fn ui_snapshot_apply_update_block_wraps_long_lines_text() {
        // This mirrors the desired layout example: sign only on first inserted line,
        // subsequent wrapped pieces start aligned under the line number gutter.
        let original = "1\n2\n3\n4\n";
        let modified = "1\nadded long line which wraps and_if_there_is_a_long_token_it_will_be_broken\n3\n4 context line which also wraps across\n";
        let patch = diffy::create_patch(original, modified).to_string();

        let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
        changes.insert(
            PathBuf::from("wrap_demo.txt"),
            FileChange::Update {
                unified_diff: patch,
                move_path: None,
            },
        );

        let lines = create_diff_summary(&changes, &PathBuf::from("/"), 28);
        snapshot_lines_text("apply_update_block_wraps_long_lines_text", &lines);
    }

    #[test]
    fn ui_snapshot_apply_update_block_line_numbers_three_digits_text() {
        let original = (1..=110).map(|i| format!("line {i}\n")).collect::<String>();
        let modified = (1..=110)
            .map(|i| {
                if i == 100 {
                    format!("line {i} changed\n")
                } else {
                    format!("line {i}\n")
                }
            })
            .collect::<String>();
        let patch = diffy::create_patch(&original, &modified).to_string();

        let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
        changes.insert(
            PathBuf::from("hundreds.txt"),
            FileChange::Update {
                unified_diff: patch,
                move_path: None,
            },
        );

        let lines = create_diff_summary(&changes, &PathBuf::from("/"), 80);
        snapshot_lines_text("apply_update_block_line_numbers_three_digits_text", &lines);
    }

    #[test]
    fn ui_snapshot_apply_update_block_relativizes_path() {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let abs_old = cwd.join("abs_old.rs");
        let abs_new = cwd.join("abs_new.rs");

        let original = "X\nY\n";
        let modified = "X changed\nY\n";
        let patch = diffy::create_patch(original, modified).to_string();

        let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
        changes.insert(
            abs_old,
            FileChange::Update {
                unified_diff: patch,
                move_path: Some(abs_new),
            },
        );

        let lines = create_diff_summary(&changes, &cwd, 80);

        snapshot_lines("apply_update_block_relativizes_path", lines, 80, 10);
    }

    #[test]
    fn syntax_highlighting_applied_to_rust_diff() {
        // Create a diff for a Rust file
        let original = "fn main() {\n    println!(\"hello\");\n}\n";
        let modified = "fn main() {\n    println!(\"world\");\n}\n";
        let patch = diffy::create_patch(original, modified).to_string();

        let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
        changes.insert(
            PathBuf::from("test.rs"),
            FileChange::Update {
                unified_diff: patch,
                move_path: None,
            },
        );

        let lines = create_diff_summary(&changes, &PathBuf::from("/"), 80);

        // Verify we got some output
        assert!(!lines.is_empty());

        // Convert to text to check content
        let text = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Should contain the function name and strings
        assert!(text.contains("main"));
        assert!(text.contains("println"));
    }

    #[test]
    fn syntax_highlighting_applied_to_python_add() {
        let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
        changes.insert(
            PathBuf::from("test.py"),
            FileChange::Add {
                content: "def hello():\n    print(\"world\")\n".to_string(),
            },
        );

        let lines = create_diff_summary(&changes, &PathBuf::from("/"), 80);

        // Verify we got output
        assert!(!lines.is_empty());

        let text = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("def"));
        assert!(text.contains("hello"));
        assert!(text.contains("print"));
    }
}
