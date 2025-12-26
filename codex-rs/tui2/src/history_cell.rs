use crate::diff_render::create_diff_summary;
use crate::diff_render::display_path_for;
use crate::exec_cell::CommandOutput;
use crate::exec_cell::OutputLinesParams;
use crate::exec_cell::TOOL_CALL_MAX_LINES;
use crate::exec_cell::TOOL_CALL_VERBOSE_MAX_LINES;
use crate::exec_cell::output_lines;
use crate::exec_cell::spinner;
use crate::exec_command::relativize_to_home;
use crate::exec_command::strip_bash_lc_and_escape;
use crate::markdown::append_markdown;
use crate::render::line_utils::line_to_static;
use crate::render::line_utils::prefix_lines;
use crate::render::line_utils::push_owned_lines;
use crate::render::renderable::Renderable;
use crate::style::user_message_style;
use crate::text_formatting::format_and_truncate_tool_result;
use crate::text_formatting::truncate_text;
use crate::tooltips;
use crate::ui_consts::LIVE_PREFIX_COLS;
use crate::update_action::UpdateAction;
use crate::version::CODEX_CLI_VERSION;
use crate::wrapping::RtOptions;
use crate::wrapping::word_wrap_line;
use crate::wrapping::word_wrap_lines;
use base64::Engine;
use codex_common::format_env_display::format_env_display;
use codex_core::config::Config;
use codex_core::config::types::McpServerTransportConfig;
use codex_core::protocol::FileChange;
use codex_core::protocol::McpAuthStatus;
use codex_core::protocol::McpInvocation;
use codex_core::protocol::SessionConfiguredEvent;
use codex_core::protocol::SubAgentBeginEvent;
use codex_core::protocol::SubAgentEndEvent;
use codex_core::protocol::SubAgentTokenUsage;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use codex_protocol::openai_models::ReasoningSummaryFormat;
use codex_protocol::protocol::SubagentHistory;
use image::DynamicImage;
use image::ImageReader;
use mcp_types::EmbeddedResourceResource;
use mcp_types::Resource;
use mcp_types::ResourceLink;
use mcp_types::ResourceTemplate;
use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use std::any::Any;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use tracing::error;
use unicode_width::UnicodeWidthStr;

/// Represents an event to display in the conversation history. Returns its
/// `Vec<Line<'static>>` representation to make it easier to display in a
/// scrollable list.
pub(crate) trait HistoryCell: std::fmt::Debug + Send + Sync + Any {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>>;

    /// Render display lines with explicit verbose flag.
    /// When verbose is true, expandable cells should show their full content.
    /// Default implementation ignores the verbose flag for backward compatibility.
    fn display_lines_verbose(&self, width: u16, verbose: bool) -> Vec<Line<'static>> {
        let _ = verbose;
        self.display_lines(width)
    }

    fn desired_height(&self, width: u16) -> u16 {
        Paragraph::new(Text::from(self.display_lines(width)))
            .wrap(Wrap { trim: false })
            .line_count(width)
            .try_into()
            .unwrap_or(0)
    }

    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.display_lines(width)
    }

    fn desired_transcript_height(&self, width: u16) -> u16 {
        let lines = self.transcript_lines(width);
        // Workaround for ratatui bug: if there's only one line and it's whitespace-only, ratatui gives 2 lines.
        if let [line] = &lines[..]
            && line
                .spans
                .iter()
                .all(|s| s.content.chars().all(char::is_whitespace))
        {
            return 1;
        }

        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .line_count(width)
            .try_into()
            .unwrap_or(0)
    }

    fn is_stream_continuation(&self) -> bool {
        false
    }
}

impl Renderable for Box<dyn HistoryCell> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let lines = self.display_lines(area.width);
        let y = if area.height == 0 {
            0
        } else {
            let overflow = lines.len().saturating_sub(usize::from(area.height));
            u16::try_from(overflow).unwrap_or(u16::MAX)
        };
        Paragraph::new(Text::from(lines))
            .scroll((y, 0))
            .render(area, buf);
    }
    fn desired_height(&self, width: u16) -> u16 {
        HistoryCell::desired_height(self.as_ref(), width)
    }
}

impl dyn HistoryCell {
    pub(crate) fn as_any(&self) -> &dyn Any {
        self
    }

    pub(crate) fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[derive(Debug)]
pub(crate) struct UserHistoryCell {
    pub message: String,
}

impl HistoryCell for UserHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();

        let wrap_width = width
            .saturating_sub(
                LIVE_PREFIX_COLS + 1, /* keep a one-column right margin for wrapping */
            )
            .max(1);

        let style = user_message_style();

        let wrapped = word_wrap_lines(
            self.message.lines().map(|l| Line::from(l).style(style)),
            // Wrap algorithm matches textarea.rs.
            RtOptions::new(usize::from(wrap_width))
                .wrap_algorithm(textwrap::WrapAlgorithm::FirstFit),
        );

        lines.push(Line::from("").style(style));
        lines.extend(prefix_lines(wrapped, "› ".bold().dim(), "  ".into()));
        lines.push(Line::from("").style(style));
        lines
    }
}

#[derive(Debug)]
pub(crate) struct ReasoningSummaryCell {
    _header: String,
    content: String,
    transcript_only: bool,
}

impl ReasoningSummaryCell {
    pub(crate) fn new(header: String, content: String, transcript_only: bool) -> Self {
        Self {
            _header: header,
            content,
            transcript_only,
        }
    }

    fn lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        append_markdown(
            &self.content,
            Some((width as usize).saturating_sub(2)),
            &mut lines,
        );
        let summary_style = Style::default().dim().italic();
        let summary_lines = lines
            .into_iter()
            .map(|mut line| {
                line.spans = line
                    .spans
                    .into_iter()
                    .map(|span| span.patch_style(summary_style))
                    .collect();
                line
            })
            .collect::<Vec<_>>();

        word_wrap_lines(
            &summary_lines,
            RtOptions::new(width as usize)
                .initial_indent("• ".dim().into())
                .subsequent_indent("  ".into()),
        )
    }
}

impl HistoryCell for ReasoningSummaryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        if self.transcript_only {
            Vec::new()
        } else {
            self.lines(width)
        }
    }

    fn display_lines_verbose(&self, width: u16, verbose: bool) -> Vec<Line<'static>> {
        // When verbose, show reasoning even if transcript_only
        if verbose || !self.transcript_only {
            self.lines(width)
        } else {
            Vec::new()
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        if self.transcript_only {
            0
        } else {
            self.lines(width).len() as u16
        }
    }

    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.lines(width)
    }

    fn desired_transcript_height(&self, width: u16) -> u16 {
        self.lines(width).len() as u16
    }
}

#[derive(Debug)]
pub(crate) struct AgentMessageCell {
    lines: Vec<Line<'static>>,
    is_first_line: bool,
}

impl AgentMessageCell {
    pub(crate) fn new(lines: Vec<Line<'static>>, is_first_line: bool) -> Self {
        Self {
            lines,
            is_first_line,
        }
    }
}

impl HistoryCell for AgentMessageCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        word_wrap_lines(
            &self.lines,
            RtOptions::new(width as usize)
                .initial_indent(if self.is_first_line {
                    "• ".dim().into()
                } else {
                    "  ".into()
                })
                .subsequent_indent("  ".into()),
        )
    }

    fn is_stream_continuation(&self) -> bool {
        !self.is_first_line
    }
}

#[derive(Debug)]
pub(crate) struct PlainHistoryCell {
    lines: Vec<Line<'static>>,
}

impl PlainHistoryCell {
    pub(crate) fn new(lines: Vec<Line<'static>>) -> Self {
        Self { lines }
    }
}

impl HistoryCell for PlainHistoryCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.lines.clone()
    }
}

#[cfg_attr(debug_assertions, allow(dead_code))]
#[derive(Debug)]
pub(crate) struct UpdateAvailableHistoryCell {
    latest_version: String,
    update_action: Option<UpdateAction>,
}

#[cfg_attr(debug_assertions, allow(dead_code))]
impl UpdateAvailableHistoryCell {
    pub(crate) fn new(latest_version: String, update_action: Option<UpdateAction>) -> Self {
        Self {
            latest_version,
            update_action,
        }
    }
}

impl HistoryCell for UpdateAvailableHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        use ratatui_macros::line;
        use ratatui_macros::text;
        let update_instruction = if let Some(update_action) = self.update_action {
            line!["Run ", update_action.command_str().cyan(), " to update."]
        } else {
            line![
                "See ",
                "https://github.com/openai/codex".cyan().underlined(),
                " for installation options."
            ]
        };

        let content = text![
            line![
                padded_emoji("✨").bold().cyan(),
                "Update available!".bold().cyan(),
                " ",
                format!("{CODEX_CLI_VERSION} -> {}", self.latest_version).bold(),
            ],
            update_instruction,
            "",
            "See full release notes:",
            "https://github.com/openai/codex/releases/latest"
                .cyan()
                .underlined(),
        ];

        let inner_width = content
            .width()
            .min(usize::from(width.saturating_sub(4)))
            .max(1);
        with_border_with_inner_width(content.lines, inner_width)
    }
}

#[derive(Debug)]
pub(crate) struct PrefixedWrappedHistoryCell {
    text: Text<'static>,
    initial_prefix: Line<'static>,
    subsequent_prefix: Line<'static>,
}

impl PrefixedWrappedHistoryCell {
    pub(crate) fn new(
        text: impl Into<Text<'static>>,
        initial_prefix: impl Into<Line<'static>>,
        subsequent_prefix: impl Into<Line<'static>>,
    ) -> Self {
        Self {
            text: text.into(),
            initial_prefix: initial_prefix.into(),
            subsequent_prefix: subsequent_prefix.into(),
        }
    }
}

impl HistoryCell for PrefixedWrappedHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        if width == 0 {
            return Vec::new();
        }
        let opts = RtOptions::new(width.max(1) as usize)
            .initial_indent(self.initial_prefix.clone())
            .subsequent_indent(self.subsequent_prefix.clone());
        let wrapped = word_wrap_lines(&self.text, opts);
        let mut out = Vec::new();
        push_owned_lines(&wrapped, &mut out);
        out
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.display_lines(width).len() as u16
    }
}

fn truncate_exec_snippet(full_cmd: &str) -> String {
    let mut snippet = match full_cmd.split_once('\n') {
        Some((first, _)) => format!("{first} ..."),
        None => full_cmd.to_string(),
    };
    snippet = truncate_text(&snippet, 80);
    snippet
}

fn exec_snippet(command: &[String]) -> String {
    let full_cmd = strip_bash_lc_and_escape(command);
    truncate_exec_snippet(&full_cmd)
}

pub fn new_approval_decision_cell(
    command: Vec<String>,
    decision: codex_core::protocol::ReviewDecision,
) -> Box<dyn HistoryCell> {
    use codex_core::protocol::ReviewDecision::*;

    let (symbol, summary): (Span<'static>, Vec<Span<'static>>) = match decision {
        Approved => {
            let snippet = Span::from(exec_snippet(&command)).dim();
            (
                "✔ ".green(),
                vec![
                    "You ".into(),
                    "approved".bold(),
                    " codex to run ".into(),
                    snippet,
                    " this time".bold(),
                ],
            )
        }
        ApprovedExecpolicyAmendment { .. } => {
            let snippet = Span::from(exec_snippet(&command)).dim();
            (
                "✔ ".green(),
                vec![
                    "You ".into(),
                    "approved".bold(),
                    " codex to run ".into(),
                    snippet,
                    " and applied the execpolicy amendment".bold(),
                ],
            )
        }
        ApprovedForSession => {
            let snippet = Span::from(exec_snippet(&command)).dim();
            (
                "✔ ".green(),
                vec![
                    "You ".into(),
                    "approved".bold(),
                    " codex to run ".into(),
                    snippet,
                    " every time this session".bold(),
                ],
            )
        }
        Denied => {
            let snippet = Span::from(exec_snippet(&command)).dim();
            (
                "✗ ".red(),
                vec![
                    "You ".into(),
                    "did not approve".bold(),
                    " codex to run ".into(),
                    snippet,
                ],
            )
        }
        Abort => {
            let snippet = Span::from(exec_snippet(&command)).dim();
            (
                "✗ ".red(),
                vec![
                    "You ".into(),
                    "canceled".bold(),
                    " the request to run ".into(),
                    snippet,
                ],
            )
        }
    };

    Box::new(PrefixedWrappedHistoryCell::new(
        Line::from(summary),
        symbol,
        "  ",
    ))
}

/// Cyan history cell line showing the current review status.
pub(crate) fn new_review_status_line(message: String) -> PlainHistoryCell {
    PlainHistoryCell {
        lines: vec![Line::from(message.cyan())],
    }
}

#[derive(Debug)]
pub(crate) struct PatchHistoryCell {
    changes: HashMap<PathBuf, FileChange>,
    cwd: PathBuf,
}

impl HistoryCell for PatchHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        create_diff_summary(&self.changes, &self.cwd, width as usize)
    }
}

#[derive(Debug)]
struct CompletedMcpToolCallWithImageOutput {
    _image: DynamicImage,
}
impl HistoryCell for CompletedMcpToolCallWithImageOutput {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        vec!["tool result (image output)".into()]
    }
}

pub(crate) const SESSION_HEADER_MAX_INNER_WIDTH: usize = 56; // Just an eyeballed value

pub(crate) fn card_inner_width(width: u16, max_inner_width: usize) -> Option<usize> {
    if width < 4 {
        return None;
    }
    let inner_width = std::cmp::min(width.saturating_sub(4) as usize, max_inner_width);
    Some(inner_width)
}

/// Render `lines` inside a border sized to the widest span in the content.
pub(crate) fn with_border(lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    with_border_internal(lines, None)
}

/// Render `lines` inside a border whose inner width is at least `inner_width`.
///
/// This is useful when callers have already clamped their content to a
/// specific width and want the border math centralized here instead of
/// duplicating padding logic in the TUI widgets themselves.
pub(crate) fn with_border_with_inner_width(
    lines: Vec<Line<'static>>,
    inner_width: usize,
) -> Vec<Line<'static>> {
    with_border_internal(lines, Some(inner_width))
}

fn with_border_internal(
    lines: Vec<Line<'static>>,
    forced_inner_width: Option<usize>,
) -> Vec<Line<'static>> {
    let max_line_width = lines
        .iter()
        .map(|line| {
            line.iter()
                .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0);
    let content_width = forced_inner_width
        .unwrap_or(max_line_width)
        .max(max_line_width);

    let mut out = Vec::with_capacity(lines.len() + 2);
    let border_inner_width = content_width + 2;
    out.push(vec![format!("╭{}╮", "─".repeat(border_inner_width)).dim()].into());

    for line in lines.into_iter() {
        let used_width: usize = line
            .iter()
            .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
            .sum();
        let span_count = line.spans.len();
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(span_count + 4);
        spans.push(Span::from("│ ").dim());
        spans.extend(line.into_iter());
        if used_width < content_width {
            spans.push(Span::from(" ".repeat(content_width - used_width)).dim());
        }
        spans.push(Span::from(" │").dim());
        out.push(Line::from(spans));
    }

    out.push(vec![format!("╰{}╯", "─".repeat(border_inner_width)).dim()].into());

    out
}

/// Return the emoji followed by a hair space (U+200A).
/// Using only the hair space avoids excessive padding after the emoji while
/// still providing a small visual gap across terminals.
pub(crate) fn padded_emoji(emoji: &str) -> String {
    format!("{emoji}\u{200A}")
}

#[derive(Debug)]
struct TooltipHistoryCell {
    tip: &'static str,
}

impl TooltipHistoryCell {
    fn new(tip: &'static str) -> Self {
        Self { tip }
    }
}

impl HistoryCell for TooltipHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let indent = "  ";
        let indent_width = UnicodeWidthStr::width(indent);
        let wrap_width = usize::from(width.max(1))
            .saturating_sub(indent_width)
            .max(1);
        let mut lines: Vec<Line<'static>> = Vec::new();
        append_markdown(
            &format!("**Tip:** {}", self.tip),
            Some(wrap_width),
            &mut lines,
        );

        prefix_lines(lines, indent.into(), indent.into())
    }
}

#[derive(Debug)]
pub struct SessionInfoCell(CompositeHistoryCell);

impl HistoryCell for SessionInfoCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.0.display_lines(width)
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.0.desired_height(width)
    }

    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.0.transcript_lines(width)
    }
}

pub(crate) fn new_session_info(
    config: &Config,
    requested_model: &str,
    event: SessionConfiguredEvent,
    is_first_event: bool,
) -> SessionInfoCell {
    let SessionConfiguredEvent {
        model,
        reasoning_effort,
        ..
    } = event;
    // Header box rendered as history (so it appears at the very top)
    let header = SessionHeaderHistoryCell::new(
        model.clone(),
        reasoning_effort,
        config.cwd.clone(),
        CODEX_CLI_VERSION,
    );
    let mut parts: Vec<Box<dyn HistoryCell>> = vec![Box::new(header)];

    if is_first_event {
        // Help lines below the header (new copy and list)
        let help_lines: Vec<Line<'static>> = vec![
            "  To get started, describe a task or try one of these commands:"
                .dim()
                .into(),
            Line::from(""),
            Line::from(vec![
                "  ".into(),
                "/init".into(),
                " - create an AGENTS.md file with instructions for Codex".dim(),
            ]),
            Line::from(vec![
                "  ".into(),
                "/status".into(),
                " - show current session configuration".dim(),
            ]),
            Line::from(vec![
                "  ".into(),
                "/approvals".into(),
                " - choose what Codex can do without approval".dim(),
            ]),
            Line::from(vec![
                "  ".into(),
                "/model".into(),
                " - choose what model and reasoning effort to use".dim(),
            ]),
            Line::from(vec![
                "  ".into(),
                "/review".into(),
                " - review any changes and find issues".dim(),
            ]),
        ];

        parts.push(Box::new(PlainHistoryCell { lines: help_lines }));
    } else {
        if config.show_tooltips
            && let Some(tooltips) = tooltips::random_tooltip().map(TooltipHistoryCell::new)
        {
            parts.push(Box::new(tooltips));
        }
        if requested_model != model {
            let lines = vec![
                "model changed:".magenta().bold().into(),
                format!("requested: {requested_model}").into(),
                format!("used: {model}").into(),
            ];
            parts.push(Box::new(PlainHistoryCell { lines }));
        }
    }

    SessionInfoCell(CompositeHistoryCell { parts })
}

pub(crate) fn new_user_prompt(message: String) -> UserHistoryCell {
    UserHistoryCell { message }
}

#[derive(Debug)]
struct SessionHeaderHistoryCell {
    version: &'static str,
    model: String,
    reasoning_effort: Option<ReasoningEffortConfig>,
    directory: PathBuf,
}

impl SessionHeaderHistoryCell {
    fn new(
        model: String,
        reasoning_effort: Option<ReasoningEffortConfig>,
        directory: PathBuf,
        version: &'static str,
    ) -> Self {
        Self {
            version,
            model,
            reasoning_effort,
            directory,
        }
    }

    fn format_directory(&self, max_width: Option<usize>) -> String {
        Self::format_directory_inner(&self.directory, max_width)
    }

    fn format_directory_inner(directory: &Path, max_width: Option<usize>) -> String {
        let formatted = if let Some(rel) = relativize_to_home(directory) {
            if rel.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~{}{}", std::path::MAIN_SEPARATOR, rel.display())
            }
        } else {
            directory.display().to_string()
        };

        if let Some(max_width) = max_width {
            if max_width == 0 {
                return String::new();
            }
            if UnicodeWidthStr::width(formatted.as_str()) > max_width {
                return crate::text_formatting::center_truncate_path(&formatted, max_width);
            }
        }

        formatted
    }

    fn reasoning_label(&self) -> Option<&'static str> {
        self.reasoning_effort.map(|effort| match effort {
            ReasoningEffortConfig::Minimal => "minimal",
            ReasoningEffortConfig::Low => "low",
            ReasoningEffortConfig::Medium => "medium",
            ReasoningEffortConfig::High => "high",
            ReasoningEffortConfig::XHigh => "xhigh",
            ReasoningEffortConfig::None => "none",
        })
    }
}

impl HistoryCell for SessionHeaderHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let Some(inner_width) = card_inner_width(width, SESSION_HEADER_MAX_INNER_WIDTH) else {
            return Vec::new();
        };

        let make_row = |spans: Vec<Span<'static>>| Line::from(spans);

        // Title line rendered inside the box: ">_ OpenAI Codex (vX)"
        let title_spans: Vec<Span<'static>> = vec![
            Span::from(">_ ").dim(),
            Span::from("OpenAI Codex").bold(),
            Span::from(" ").dim(),
            Span::from(format!("(v{})", self.version)).dim(),
        ];

        const CHANGE_MODEL_HINT_COMMAND: &str = "/model";
        const CHANGE_MODEL_HINT_EXPLANATION: &str = " to change";
        const DIR_LABEL: &str = "directory:";
        let label_width = DIR_LABEL.len();
        let model_label = format!(
            "{model_label:<label_width$}",
            model_label = "model:",
            label_width = label_width
        );
        let reasoning_label = self.reasoning_label();
        let mut model_spans: Vec<Span<'static>> = vec![
            Span::from(format!("{model_label} ")).dim(),
            Span::from(self.model.clone()),
        ];
        if let Some(reasoning) = reasoning_label {
            model_spans.push(Span::from(" "));
            model_spans.push(Span::from(reasoning));
        }
        model_spans.push("   ".dim());
        model_spans.push(CHANGE_MODEL_HINT_COMMAND.cyan());
        model_spans.push(CHANGE_MODEL_HINT_EXPLANATION.dim());

        let dir_label = format!("{DIR_LABEL:<label_width$}");
        let dir_prefix = format!("{dir_label} ");
        let dir_prefix_width = UnicodeWidthStr::width(dir_prefix.as_str());
        let dir_max_width = inner_width.saturating_sub(dir_prefix_width);
        let dir = self.format_directory(Some(dir_max_width));
        let dir_spans = vec![Span::from(dir_prefix).dim(), Span::from(dir)];

        let lines = vec![
            make_row(title_spans),
            make_row(Vec::new()),
            make_row(model_spans),
            make_row(dir_spans),
        ];

        with_border(lines)
    }
}

#[derive(Debug)]
pub(crate) struct CompositeHistoryCell {
    parts: Vec<Box<dyn HistoryCell>>,
}

impl CompositeHistoryCell {
    pub(crate) fn new(parts: Vec<Box<dyn HistoryCell>>) -> Self {
        Self { parts }
    }
}

impl HistoryCell for CompositeHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut out: Vec<Line<'static>> = Vec::new();
        let mut first = true;
        for part in &self.parts {
            let mut lines = part.display_lines(width);
            if !lines.is_empty() {
                if !first {
                    out.push(Line::from(""));
                }
                out.append(&mut lines);
                first = false;
            }
        }
        out
    }
}

#[derive(Debug)]
pub(crate) struct McpToolCallCell {
    call_id: String,
    invocation: McpInvocation,
    start_time: Instant,
    duration: Option<Duration>,
    result: Option<Result<mcp_types::CallToolResult, String>>,
    animations_enabled: bool,
}

impl McpToolCallCell {
    pub(crate) fn new(
        call_id: String,
        invocation: McpInvocation,
        animations_enabled: bool,
    ) -> Self {
        Self {
            call_id,
            invocation,
            start_time: Instant::now(),
            duration: None,
            result: None,
            animations_enabled,
        }
    }

    pub(crate) fn call_id(&self) -> &str {
        &self.call_id
    }

    pub(crate) fn complete(
        &mut self,
        duration: Duration,
        result: Result<mcp_types::CallToolResult, String>,
    ) -> Option<Box<dyn HistoryCell>> {
        let image_cell = try_new_completed_mcp_tool_call_with_image_output(&result)
            .map(|cell| Box::new(cell) as Box<dyn HistoryCell>);
        self.duration = Some(duration);
        self.result = Some(result);
        image_cell
    }

    fn success(&self) -> Option<bool> {
        match self.result.as_ref() {
            Some(Ok(result)) => Some(!result.is_error.unwrap_or(false)),
            Some(Err(_)) => Some(false),
            None => None,
        }
    }

    pub(crate) fn mark_failed(&mut self) {
        let elapsed = self.start_time.elapsed();
        self.duration = Some(elapsed);
        self.result = Some(Err("interrupted".to_string()));
    }

    fn render_content_block(block: &mcp_types::ContentBlock, width: usize, max_lines: usize) -> String {
        match block {
            mcp_types::ContentBlock::TextContent(text) => {
                format_and_truncate_tool_result(&text.text, max_lines, width)
            }
            mcp_types::ContentBlock::ImageContent(_) => "<image content>".to_string(),
            mcp_types::ContentBlock::AudioContent(_) => "<audio content>".to_string(),
            mcp_types::ContentBlock::EmbeddedResource(resource) => {
                let uri = match &resource.resource {
                    EmbeddedResourceResource::TextResourceContents(text) => text.uri.clone(),
                    EmbeddedResourceResource::BlobResourceContents(blob) => blob.uri.clone(),
                };
                format!("embedded resource: {uri}")
            }
            mcp_types::ContentBlock::ResourceLink(ResourceLink { uri, .. }) => {
                format!("link: {uri}")
            }
        }
    }
}

impl McpToolCallCell {
    /// Internal rendering with explicit max_lines limit for tool result truncation.
    fn render_with_max_lines(&self, width: u16, max_lines: usize) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        let status = self.success();
        let bullet = match status {
            Some(true) => "•".green().bold(),
            Some(false) => "•".red().bold(),
            None => spinner(Some(self.start_time), self.animations_enabled),
        };
        let header_text = if status.is_some() {
            "Called"
        } else {
            "Calling"
        };

        let invocation_line = line_to_static(&format_mcp_invocation(self.invocation.clone()));
        let mut compact_spans = vec![bullet.clone(), " ".into(), header_text.bold(), " ".into()];
        let mut compact_header = Line::from(compact_spans.clone());
        let reserved = compact_header.width();

        let inline_invocation =
            invocation_line.width() <= (width as usize).saturating_sub(reserved);

        if inline_invocation {
            compact_header.extend(invocation_line.spans.clone());
            lines.push(compact_header);
        } else {
            compact_spans.pop(); // drop trailing space for standalone header
            lines.push(Line::from(compact_spans));

            let opts = RtOptions::new((width as usize).saturating_sub(4))
                .initial_indent("".into())
                .subsequent_indent("    ".into());
            let wrapped = word_wrap_line(&invocation_line, opts);
            let body_lines: Vec<Line<'static>> = wrapped.iter().map(line_to_static).collect();
            lines.extend(prefix_lines(body_lines, "  └ ".dim(), "    ".into()));
        }

        let mut detail_lines: Vec<Line<'static>> = Vec::new();
        // Reserve four columns for the tree prefix ("  └ "/"    ") and ensure the wrapper still has at least one cell to work with.
        let detail_wrap_width = (width as usize).saturating_sub(4).max(1);

        if let Some(result) = &self.result {
            match result {
                Ok(mcp_types::CallToolResult { content, .. }) => {
                    if !content.is_empty() {
                        for block in content {
                            let text = Self::render_content_block(block, detail_wrap_width, max_lines);
                            for segment in text.split('\n') {
                                let line = Line::from(segment.to_string().dim());
                                let wrapped = word_wrap_line(
                                    &line,
                                    RtOptions::new(detail_wrap_width)
                                        .initial_indent("".into())
                                        .subsequent_indent("    ".into()),
                                );
                                detail_lines.extend(wrapped.iter().map(line_to_static));
                            }
                        }
                    }
                }
                Err(err) => {
                    let err_text = format_and_truncate_tool_result(
                        &format!("Error: {err}"),
                        max_lines,
                        width as usize,
                    );
                    let err_line = Line::from(err_text.dim());
                    let wrapped = word_wrap_line(
                        &err_line,
                        RtOptions::new(detail_wrap_width)
                            .initial_indent("".into())
                            .subsequent_indent("    ".into()),
                    );
                    detail_lines.extend(wrapped.iter().map(line_to_static));
                }
            }
        }

        if !detail_lines.is_empty() {
            let initial_prefix: Span<'static> = if inline_invocation {
                "  └ ".dim()
            } else {
                "    ".into()
            };
            lines.extend(prefix_lines(detail_lines, initial_prefix, "    ".into()));
        }

        lines
    }
}

impl HistoryCell for McpToolCallCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.render_with_max_lines(width, TOOL_CALL_MAX_LINES)
    }

    fn display_lines_verbose(&self, width: u16, verbose: bool) -> Vec<Line<'static>> {
        let max_lines = if verbose {
            TOOL_CALL_VERBOSE_MAX_LINES
        } else {
            TOOL_CALL_MAX_LINES
        };
        self.render_with_max_lines(width, max_lines)
    }
}

pub(crate) fn new_active_mcp_tool_call(
    call_id: String,
    invocation: McpInvocation,
    animations_enabled: bool,
) -> McpToolCallCell {
    McpToolCallCell::new(call_id, invocation, animations_enabled)
}

pub(crate) fn new_web_search_call(query: String) -> PrefixedWrappedHistoryCell {
    let text: Text<'static> = Line::from(vec!["Searched".bold(), " ".into(), query.into()]).into();
    PrefixedWrappedHistoryCell::new(text, "• ".dim(), "  ")
}

/// If the first content is an image, return a new cell with the image.
/// TODO(rgwood-dd): Handle images properly even if they're not the first result.
fn try_new_completed_mcp_tool_call_with_image_output(
    result: &Result<mcp_types::CallToolResult, String>,
) -> Option<CompletedMcpToolCallWithImageOutput> {
    match result {
        Ok(mcp_types::CallToolResult { content, .. }) => {
            if let Some(mcp_types::ContentBlock::ImageContent(image)) = content.first() {
                let raw_data = match base64::engine::general_purpose::STANDARD.decode(&image.data) {
                    Ok(data) => data,
                    Err(e) => {
                        error!("Failed to decode image data: {e}");
                        return None;
                    }
                };
                let reader = match ImageReader::new(Cursor::new(raw_data)).with_guessed_format() {
                    Ok(reader) => reader,
                    Err(e) => {
                        error!("Failed to guess image format: {e}");
                        return None;
                    }
                };

                let image = match reader.decode() {
                    Ok(image) => image,
                    Err(e) => {
                        error!("Image decoding failed: {e}");
                        return None;
                    }
                };

                Some(CompletedMcpToolCallWithImageOutput { _image: image })
            } else {
                None
            }
        }
        _ => None,
    }
}

#[allow(clippy::disallowed_methods)]
pub(crate) fn new_warning_event(message: String) -> PrefixedWrappedHistoryCell {
    PrefixedWrappedHistoryCell::new(message.yellow(), "⚠ ".yellow(), "  ")
}

#[derive(Debug)]
pub(crate) struct DeprecationNoticeCell {
    summary: String,
    details: Option<String>,
}

pub(crate) fn new_deprecation_notice(
    summary: String,
    details: Option<String>,
) -> DeprecationNoticeCell {
    DeprecationNoticeCell { summary, details }
}

impl HistoryCell for DeprecationNoticeCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        lines.push(vec!["⚠ ".red().bold(), self.summary.clone().red()].into());

        let wrap_width = width.saturating_sub(4).max(1) as usize;

        if let Some(details) = &self.details {
            let line = textwrap::wrap(details, wrap_width)
                .into_iter()
                .map(|s| s.to_string().dim().into())
                .collect::<Vec<_>>();
            lines.extend(line);
        }

        lines
    }
}

/// Render a summary of configured MCP servers from the current `Config`.
pub(crate) fn empty_mcp_output() -> PlainHistoryCell {
    let lines: Vec<Line<'static>> = vec![
        "/mcp".magenta().into(),
        "".into(),
        vec!["🔌  ".into(), "MCP Tools".bold()].into(),
        "".into(),
        "  • No MCP servers configured.".italic().into(),
        Line::from(vec![
            "    See the ".into(),
            "\u{1b}]8;;https://github.com/openai/codex/blob/main/docs/config.md#mcp_servers\u{7}MCP docs\u{1b}]8;;\u{7}".underlined(),
            " to configure them.".into(),
        ])
        .style(Style::default().add_modifier(Modifier::DIM)),
    ];

    PlainHistoryCell { lines }
}

/// Render MCP tools grouped by connection using the fully-qualified tool names.
pub(crate) fn new_mcp_tools_output(
    config: &Config,
    tools: HashMap<String, mcp_types::Tool>,
    resources: HashMap<String, Vec<Resource>>,
    resource_templates: HashMap<String, Vec<ResourceTemplate>>,
    auth_statuses: &HashMap<String, McpAuthStatus>,
) -> PlainHistoryCell {
    let mut lines: Vec<Line<'static>> = vec![
        "/mcp".magenta().into(),
        "".into(),
        vec!["🔌  ".into(), "MCP Tools".bold()].into(),
        "".into(),
    ];

    if tools.is_empty() {
        lines.push("  • No MCP tools available.".italic().into());
        lines.push("".into());
        return PlainHistoryCell { lines };
    }

    let mut servers: Vec<_> = config.mcp_servers.iter().collect();
    servers.sort_by(|(a, _), (b, _)| a.cmp(b));

    for (server, cfg) in servers {
        let prefix = format!("mcp__{server}__");
        let mut names: Vec<String> = tools
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .map(|k| k[prefix.len()..].to_string())
            .collect();
        names.sort();

        let auth_status = auth_statuses
            .get(server.as_str())
            .copied()
            .unwrap_or(McpAuthStatus::Unsupported);
        let mut header: Vec<Span<'static>> = vec!["  • ".into(), server.clone().into()];
        if !cfg.enabled {
            header.push(" ".into());
            header.push("(disabled)".red());
            lines.push(header.into());
            lines.push(Line::from(""));
            continue;
        }
        lines.push(header.into());
        lines.push(vec!["    • Status: ".into(), "enabled".green()].into());
        lines.push(vec!["    • Auth: ".into(), auth_status.to_string().into()].into());

        match &cfg.transport {
            McpServerTransportConfig::Stdio {
                command,
                args,
                env,
                env_vars,
                cwd,
            } => {
                let args_suffix = if args.is_empty() {
                    String::new()
                } else {
                    format!(" {}", args.join(" "))
                };
                let cmd_display = format!("{command}{args_suffix}");
                lines.push(vec!["    • Command: ".into(), cmd_display.into()].into());

                if let Some(cwd) = cwd.as_ref() {
                    lines.push(vec!["    • Cwd: ".into(), cwd.display().to_string().into()].into());
                }

                let env_display = format_env_display(env.as_ref(), env_vars);
                if env_display != "-" {
                    lines.push(vec!["    • Env: ".into(), env_display.into()].into());
                }
            }
            McpServerTransportConfig::StreamableHttp {
                url,
                http_headers,
                env_http_headers,
                ..
            } => {
                lines.push(vec!["    • URL: ".into(), url.clone().into()].into());
                if let Some(headers) = http_headers.as_ref()
                    && !headers.is_empty()
                {
                    let mut pairs: Vec<_> = headers.iter().collect();
                    pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
                    let display = pairs
                        .into_iter()
                        .map(|(name, _)| format!("{name}=*****"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    lines.push(vec!["    • HTTP headers: ".into(), display.into()].into());
                }
                if let Some(headers) = env_http_headers.as_ref()
                    && !headers.is_empty()
                {
                    let mut pairs: Vec<_> = headers.iter().collect();
                    pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
                    let display = pairs
                        .into_iter()
                        .map(|(name, var)| format!("{name}={var}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    lines.push(vec!["    • Env HTTP headers: ".into(), display.into()].into());
                }
            }
        }

        if names.is_empty() {
            lines.push("    • Tools: (none)".into());
        } else {
            lines.push(vec!["    • Tools: ".into(), names.join(", ").into()].into());
        }

        let server_resources: Vec<Resource> =
            resources.get(server.as_str()).cloned().unwrap_or_default();
        if server_resources.is_empty() {
            lines.push("    • Resources: (none)".into());
        } else {
            let mut spans: Vec<Span<'static>> = vec!["    • Resources: ".into()];

            for (idx, resource) in server_resources.iter().enumerate() {
                if idx > 0 {
                    spans.push(", ".into());
                }

                let label = resource.title.as_ref().unwrap_or(&resource.name);
                spans.push(label.clone().into());
                spans.push(" ".into());
                spans.push(format!("({})", resource.uri).dim());
            }

            lines.push(spans.into());
        }

        let server_templates: Vec<ResourceTemplate> = resource_templates
            .get(server.as_str())
            .cloned()
            .unwrap_or_default();
        if server_templates.is_empty() {
            lines.push("    • Resource templates: (none)".into());
        } else {
            let mut spans: Vec<Span<'static>> = vec!["    • Resource templates: ".into()];

            for (idx, template) in server_templates.iter().enumerate() {
                if idx > 0 {
                    spans.push(", ".into());
                }

                let label = template.title.as_ref().unwrap_or(&template.name);
                spans.push(label.clone().into());
                spans.push(" ".into());
                spans.push(format!("({})", template.uri_template).dim());
            }

            lines.push(spans.into());
        }

        lines.push(Line::from(""));
    }

    PlainHistoryCell { lines }
}
pub(crate) fn new_info_event(message: String, hint: Option<String>) -> PlainHistoryCell {
    let mut line = vec!["• ".dim(), message.into()];
    if let Some(hint) = hint {
        line.push(" ".into());
        line.push(hint.dark_gray());
    }
    let lines: Vec<Line<'static>> = vec![line.into()];
    PlainHistoryCell { lines }
}

pub(crate) fn new_error_event(message: String) -> PlainHistoryCell {
    // Use a hair space (U+200A) to create a subtle, near-invisible separation
    // before the text. VS16 is intentionally omitted to keep spacing tighter
    // in terminals like Ghostty.
    let lines: Vec<Line<'static>> = vec![vec![format!("■ {message}").red()].into()];
    PlainHistoryCell { lines }
}

/// Create a new `PendingPatch` cell that lists the file‑level summary of
/// a proposed patch. The summary lines should already be formatted (e.g.
/// "A path/to/file.rs").
pub(crate) fn new_patch_event(
    changes: HashMap<PathBuf, FileChange>,
    cwd: &Path,
) -> PatchHistoryCell {
    PatchHistoryCell {
        changes,
        cwd: cwd.to_path_buf(),
    }
}

pub(crate) fn new_patch_apply_failure(stderr: String) -> PlainHistoryCell {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Failure title
    lines.push(Line::from("✘ Failed to apply patch".magenta().bold()));

    if !stderr.trim().is_empty() {
        let output = output_lines(
            Some(&CommandOutput {
                exit_code: 1,
                formatted_output: String::new(),
                aggregated_output: stderr,
            }),
            OutputLinesParams {
                line_limit: TOOL_CALL_MAX_LINES,
                only_err: true,
                include_angle_pipe: true,
                include_prefix: true,
            },
        );
        lines.extend(output.lines);
    }

    PlainHistoryCell { lines }
}

pub(crate) fn new_view_image_tool_call(path: PathBuf, cwd: &Path) -> PlainHistoryCell {
    let display_path = display_path_for(&path, cwd);

    let lines: Vec<Line<'static>> = vec![
        vec!["• ".dim(), "Viewed Image".bold()].into(),
        vec!["  └ ".dim(), display_path.dim()].into(),
    ];

    PlainHistoryCell { lines }
}

pub(crate) fn new_reasoning_summary_block(
    full_reasoning_buffer: String,
    reasoning_summary_format: ReasoningSummaryFormat,
) -> Box<dyn HistoryCell> {
    if reasoning_summary_format == ReasoningSummaryFormat::Experimental {
        // Experimental format is following:
        // ** header **
        //
        // reasoning summary
        //
        // So we need to strip header from reasoning summary
        let full_reasoning_buffer = full_reasoning_buffer.trim();
        if let Some(open) = full_reasoning_buffer.find("**") {
            let after_open = &full_reasoning_buffer[(open + 2)..];
            if let Some(close) = after_open.find("**") {
                let after_close_idx = open + 2 + close + 2;
                // if we don't have anything beyond `after_close_idx`
                // then we don't have a summary to inject into history
                if after_close_idx < full_reasoning_buffer.len() {
                    let header_buffer = full_reasoning_buffer[..after_close_idx].to_string();
                    let summary_buffer = full_reasoning_buffer[after_close_idx..].to_string();
                    return Box::new(ReasoningSummaryCell::new(
                        header_buffer,
                        summary_buffer,
                        false,
                    ));
                }
            }
        }
    }
    Box::new(ReasoningSummaryCell::new(
        "".to_string(),
        full_reasoning_buffer,
        true,
    ))
}

#[derive(Debug)]
pub struct FinalMessageSeparator {
    elapsed_seconds: Option<u64>,
}
impl FinalMessageSeparator {
    pub(crate) fn new(elapsed_seconds: Option<u64>) -> Self {
        Self { elapsed_seconds }
    }
}
impl HistoryCell for FinalMessageSeparator {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let elapsed_seconds = self
            .elapsed_seconds
            .map(super::status_indicator_widget::fmt_elapsed_compact);
        if let Some(elapsed_seconds) = elapsed_seconds {
            let worked_for = format!("─ Worked for {elapsed_seconds} ─");
            let worked_for_width = worked_for.width();
            vec![
                Line::from_iter([
                    worked_for,
                    "─".repeat((width as usize).saturating_sub(worked_for_width)),
                ])
                .dim(),
            ]
        } else {
            vec![Line::from_iter(["─".repeat(width as usize).dim()])]
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SubAgentCell - Compact view for sub-agent tasks
// ─────────────────────────────────────────────────────────────────────────────

/// Status of a sub-agent task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubAgentStatus {
    Running,
    Completed,
    Error,
}

/// A forwarded tool event from a sub-agent for expanded view.
#[derive(Debug, Clone)]
pub(crate) struct ForwardedToolEvent {
    pub tool_name: String,
    pub title: Option<String>,
    pub status: String,
}

/// Cell displaying a sub-agent task with compact view.
#[derive(Debug)]
pub(crate) struct SubAgentCell {
    session_id: String,
    agent_type: String,
    description: String,
    status: SubAgentStatus,
    start_time: Option<Instant>,
    /// Whether this is a resumed session.
    resumed: bool,

    // Statistics
    tool_uses_count: usize,
    token_usage: Option<SubAgentTokenUsage>,
    duration_ms: Option<u64>,

    // Forwarded events for expanded view (transient)
    forwarded_events: Vec<ForwardedToolEvent>,

    // UI state
    expanded: bool,
    animations_enabled: bool,
}

impl SubAgentCell {
    /// Format token usage in a human-readable format.
    pub fn format_tokens(&self) -> String {
        match &self.token_usage {
            Some(usage) if usage.total_tokens > 0 => {
                let k = usage.total_tokens as f64 / 1000.0;
                if k >= 1.0 {
                    format!("{k:.1}k tokens")
                } else {
                    let tokens = usage.total_tokens;
                    format!("{tokens} tokens")
                }
            }
            _ => "-- tokens".to_string(),
        }
    }

    /// Add a forwarded tool event.
    pub fn add_forwarded_event(&mut self, event: ForwardedToolEvent) {
        self.forwarded_events.push(event);
        self.tool_uses_count = self.forwarded_events.len();
    }

    /// Mark the sub-agent as complete with final statistics.
    pub fn complete(&mut self, end_event: &SubAgentEndEvent) {
        self.status = if end_event.success {
            SubAgentStatus::Completed
        } else {
            SubAgentStatus::Error
        };
        self.duration_ms = Some(end_event.duration_ms);
        self.token_usage = end_event.token_usage.clone();
        self.tool_uses_count = end_event.tool_summary.len();
        self.start_time = None;
    }

    /// Get the session ID for this sub-agent.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Get the agent type.
    pub fn agent_type(&self) -> &str {
        &self.agent_type
    }

    /// Get the list of forwarded events.
    pub fn forwarded_events(&self) -> &[ForwardedToolEvent] {
        &self.forwarded_events
    }

    /// Update the status of the last forwarded event.
    pub fn update_last_event_status(&mut self, status: &str) {
        if let Some(last) = self.forwarded_events.last_mut() {
            last.status = status.to_string();
        }
    }

    /// Accumulate token usage from a forwarded TokenCount event.
    ///
    /// This allows streaming token updates as the sub-agent runs,
    /// rather than only showing tokens at completion.
    pub fn accumulate_tokens(&mut self, input_delta: u64, output_delta: u64) {
        let usage = self.token_usage.get_or_insert(SubAgentTokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
        });
        usage.input_tokens = usage.input_tokens.saturating_add(input_delta);
        usage.output_tokens = usage.output_tokens.saturating_add(output_delta);
        usage.total_tokens = usage.input_tokens + usage.output_tokens;
    }

    /// Get the current status.
    pub fn status(&self) -> SubAgentStatus {
        self.status
    }

    /// Get the start time.
    pub fn start_time(&self) -> Option<Instant> {
        self.start_time
    }

    /// Get the description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Get the tool uses count.
    pub fn tool_uses_count(&self) -> usize {
        self.tool_uses_count
    }

    /// Get a human-readable status text for the current state.
    pub fn current_status_text(&self) -> String {
        if self.forwarded_events.is_empty() {
            "Initializing...".to_string()
        } else if let Some(last) = self.forwarded_events.last() {
            match last.status.as_str() {
                "running" => format!("Running {}...", last.tool_name),
                "completed" => format!("Completed {}", last.tool_name),
                "error" => format!("Error in {}", last.tool_name),
                _ => last.status.clone(),
            }
        } else {
            "Working...".to_string()
        }
    }

    /// Toggle the expanded state of the cell.
    #[allow(dead_code)] // Used in tests
    pub fn toggle_expanded(&mut self) {
        self.expanded = !self.expanded;
    }
}

impl HistoryCell for SubAgentCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        // Use per-cell expanded state for backward compatibility
        self.render_with_expanded(width, self.expanded)
    }

    fn display_lines_verbose(&self, width: u16, verbose: bool) -> Vec<Line<'static>> {
        // Use global verbose mode OR per-cell expanded state
        self.render_with_expanded(width, verbose || self.expanded)
    }

    fn desired_height(&self, _width: u16) -> u16 {
        self.calculate_height(self.expanded)
    }
}

impl SubAgentCell {
    /// Internal rendering with explicit expanded flag.
    fn render_with_expanded(&self, width: u16, show_expanded: bool) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();

        // Status bullet
        let bullet = match self.status {
            SubAgentStatus::Completed => "●".green().bold(),
            SubAgentStatus::Error => "●".red().bold(),
            SubAgentStatus::Running => spinner(Some(self.start_time.unwrap_or(Instant::now())), self.animations_enabled),
        };

        let status_text = match (&self.status, self.resumed) {
            (SubAgentStatus::Running, true) => "Resumed",
            (SubAgentStatus::Running, false) => "Running",
            (SubAgentStatus::Completed, _) => "Done",
            (SubAgentStatus::Error, _) => "Error",
        };

        // Format: "● agent-type · Status · N tool uses · XXk tokens"
        let header = format!(
            "{} · {} · {} tool uses · {}",
            self.agent_type,
            status_text,
            self.tool_uses_count,
            self.format_tokens(),
        );

        lines.push(Line::from(vec![
            bullet,
            " ".into(),
            header.into(),
        ]));

        // Description line
        let desc_width = (width as usize).saturating_sub(4).max(1);
        let truncated_desc = if self.description.len() > desc_width {
            format!("{}…", &self.description[..desc_width.saturating_sub(1)])
        } else {
            self.description.clone()
        };
        lines.push(Line::from(vec![
            "  └ ".dim(),
            truncated_desc.dim(),
        ]));

        // If expanded, show forwarded events
        if show_expanded && !self.forwarded_events.is_empty() {
            for (i, event) in self.forwarded_events.iter().enumerate() {
                let prefix = if i == self.forwarded_events.len() - 1 {
                    "    └ "
                } else {
                    "    ├ "
                };
                let status_icon = match event.status.as_str() {
                    "completed" => "✓".green(),
                    "error" => "✗".red(),
                    _ => "○".cyan(),
                };
                let title_text = event.title.clone().unwrap_or_default();
                lines.push(Line::from(vec![
                    prefix.dim(),
                    status_icon,
                    " ".into(),
                    event.tool_name.clone().into(),
                    " ".into(),
                    title_text.dim(),
                ]));
            }
        } else if show_expanded {
            lines.push(Line::from(vec![
                "    └ ".dim(),
                "(no tool calls)".dim().italic(),
            ]));
        }

        // Show expand/collapse hint
        if !self.forwarded_events.is_empty() || show_expanded {
            let hint = if show_expanded {
                "(ctrl+o to collapse)"
            } else {
                "(ctrl+o to expand)"
            };
            lines.push(Line::from(hint.dim()));
        }

        lines
    }

    /// Calculate height with explicit expanded flag.
    fn calculate_height(&self, show_expanded: bool) -> u16 {
        let base = 2; // Header + description
        let hint = if !self.forwarded_events.is_empty() || show_expanded { 1 } else { 0 };
        if show_expanded {
            let events = if self.forwarded_events.is_empty() { 1 } else { self.forwarded_events.len() };
            base + events as u16 + hint
        } else {
            base + hint
        }
    }
}

/// Create a new SubAgentCell from a SubAgentBeginEvent.
pub(crate) fn new_subagent_cell(
    begin_event: SubAgentBeginEvent,
    animations_enabled: bool,
) -> SubAgentCell {
    SubAgentCell {
        session_id: begin_event.session_id,
        agent_type: begin_event.agent_type,
        description: begin_event.description,
        status: SubAgentStatus::Running,
        start_time: Some(Instant::now()),
        resumed: begin_event.resumed,
        tool_uses_count: 0,
        token_usage: None,
        duration_ms: None,
        forwarded_events: Vec::new(),
        expanded: false,
        animations_enabled,
    }
}

/// Create a SubAgentCell from SubagentHistory (for session resume).
/// The cell is marked as completed and reconstructs statistics from the history.
pub(crate) fn subagent_cell_from_history(
    history: SubagentHistory,
    animations_enabled: bool,
) -> SubAgentCell {
    use codex_core::protocol::EventMsg;
    use codex_protocol::protocol::RolloutItem;

    // Extract SubAgentBegin and SubAgentEnd events from history
    let mut begin_event: Option<SubAgentBeginEvent> = None;
    let mut end_event: Option<SubAgentEndEvent> = None;

    for item in &history.history {
        if let RolloutItem::EventMsg(ev) = item {
            match ev {
                EventMsg::SubAgentBegin(ev) => {
                    begin_event = Some(ev.clone());
                }
                EventMsg::SubAgentEnd(ev) => {
                    end_event = Some(ev.clone());
                }
                _ => {}
            }
        }
    }

    // Build the cell from the events we found
    let (agent_type, description, resumed) = begin_event
        .map(|ev| (ev.agent_type, ev.description, ev.resumed))
        .unwrap_or_else(|| {
            // Fallback to history metadata if no begin event found
            (
                history.agent_type.clone(),
                history.description.clone(),
                true,
            )
        });

    // Extract final statistics from end event if available
    let (status, tool_uses_count, token_usage, duration_ms) = end_event
        .map(|ev| {
            let status = if ev.success {
                SubAgentStatus::Completed
            } else {
                SubAgentStatus::Error
            };
            (status, ev.tool_summary.len(), ev.token_usage, Some(ev.duration_ms))
        })
        .unwrap_or_else(|| {
            // If no end event, assume completed successfully
            (SubAgentStatus::Completed, 0, None, None)
        });

    SubAgentCell {
        session_id: history.session_id,
        agent_type,
        description,
        status,
        start_time: None,
        resumed,
        tool_uses_count,
        token_usage,
        duration_ms,
        forwarded_events: Vec::new(),
        expanded: false,
        animations_enabled,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// RunningAgentsGroup - Grouped view for parallel running agents
// ─────────────────────────────────────────────────────────────────────────────

/// Groups multiple running sub-agents under a single collapsible header.
/// Used for displaying parallel running agents in a compact tree view.
pub(crate) struct RunningAgentsGroup<'a> {
    agents: Vec<&'a SubAgentCell>,
    expanded: bool,
    animations_enabled: bool,
}

impl<'a> RunningAgentsGroup<'a> {
    /// Create a new group from a collection of sub-agent cells.
    pub fn new(agents: Vec<&'a SubAgentCell>, expanded: bool, animations_enabled: bool) -> Self {
        Self {
            agents,
            expanded,
            animations_enabled,
        }
    }

    /// Returns true if there are no agents to display.
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }

    /// Render the group as a list of lines.
    pub fn render_lines(&self, width: u16) -> Vec<Line<'static>> {
        if self.agents.is_empty() {
            return Vec::new();
        }

        let mut lines: Vec<Line<'static>> = Vec::new();

        // Count running vs completed agents
        let running_count = self.agents.iter().filter(|a| a.status() == SubAgentStatus::Running).count();
        let total_count = self.agents.len();
        let completed_count = total_count - running_count;

        // Group header with running/total info
        let bullet = spinner(None, self.animations_enabled);
        let hint = if self.expanded {
            "(ctrl+o to collapse)"
        } else {
            "(ctrl+o to expand)"
        };

        let header_text = if running_count == 0 {
            // All completed, waiting to be moved to history
            format!(
                "{} {} completed {}",
                total_count,
                if total_count == 1 { "agent" } else { "agents" },
                hint
            )
        } else if completed_count == 0 {
            // All still running
            format!(
                "Running {} {}... {}",
                total_count,
                if total_count == 1 { "agent" } else { "agents" },
                hint
            )
        } else {
            // Mixed: some running, some completed
            format!("Running {running_count} of {total_count} agents... {hint}")
        };

        lines.push(Line::from(vec![bullet, " ".into(), header_text.into()]));

        // Render each agent with tree prefixes
        let agent_count = self.agents.len();
        for (i, agent) in self.agents.iter().enumerate() {
            let is_last = i == agent_count - 1;
            self.render_agent_in_group(&mut lines, agent, is_last, width);
        }

        lines
    }

    /// Render a single agent within the group with tree prefixes.
    fn render_agent_in_group(
        &self,
        lines: &mut Vec<Line<'static>>,
        agent: &SubAgentCell,
        is_last: bool,
        width: u16,
    ) {
        // Tree prefix for agent header
        let tree_prefix = if is_last { "└─ " } else { "├─ " };
        let continuation = if is_last { "   " } else { "│  " };

        // Agent bullet based on status
        let agent_bullet = match agent.status() {
            SubAgentStatus::Running => spinner(agent.start_time(), self.animations_enabled),
            SubAgentStatus::Completed => "●".green().bold(),
            SubAgentStatus::Error => "●".red().bold(),
        };

        // Agent header: "agent-type (description) · N tool uses"
        let desc = truncate_description(agent.description(), (width as usize).saturating_sub(30));
        let header = format!(
            "{} ({}) · {} tool uses",
            agent.agent_type(),
            desc,
            agent.tool_uses_count(),
        );

        lines.push(Line::from(vec![
            tree_prefix.dim(),
            agent_bullet,
            " ".into(),
            header.into(),
        ]));

        // Status line under agent
        let status_prefix = format!("{continuation}└ ");
        let status_text = agent.current_status_text();
        lines.push(Line::from(vec![status_prefix.dim(), status_text.dim()]));

        // If expanded, show forwarded events
        if self.expanded {
            let events = agent.forwarded_events();
            if events.is_empty() {
                // No events yet
            } else {
                let event_count = events.len();
                for (j, event) in events.iter().enumerate() {
                    let is_last_event = j == event_count - 1;
                    let event_prefix = format!(
                        "{}   {} ",
                        continuation,
                        if is_last_event { "└" } else { "├" }
                    );

                    let status_icon = match event.status.as_str() {
                        "completed" => "✓".green(),
                        "error" => "✗".red(),
                        _ => "○".cyan(),
                    };

                    let title_text = event.title.clone().unwrap_or_default();
                    lines.push(Line::from(vec![
                        event_prefix.dim(),
                        status_icon,
                        " ".into(),
                        event.tool_name.clone().into(),
                        " ".into(),
                        title_text.dim(),
                    ]));
                }
            }
        }
    }

    /// Calculate the height needed to render the group.
    pub fn calculate_height(&self) -> u16 {
        if self.agents.is_empty() {
            return 0;
        }

        let mut height: u16 = 1; // Group header

        for agent in &self.agents {
            height += 2; // Agent header + status line
            if self.expanded && !agent.forwarded_events().is_empty() {
                height += agent.forwarded_events().len() as u16;
            }
        }

        height
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SubAgentGroupCell - History cell for grouped completed agents
// ─────────────────────────────────────────────────────────────────────────────

/// A history cell that groups multiple completed sub-agents together.
/// Used to display completed parallel agents in a grouped view.
#[derive(Debug)]
pub(crate) struct SubAgentGroupCell {
    cells: Vec<SubAgentCell>,
    expanded: bool,
}

impl SubAgentGroupCell {
    /// Create a new group from a collection of completed sub-agent cells.
    pub fn new(cells: Vec<SubAgentCell>) -> Self {
        Self {
            cells,
            expanded: false,
        }
    }

    /// Render the group with an explicit expanded flag.
    fn render_with_expanded(&self, width: u16, show_expanded: bool) -> Vec<Line<'static>> {
        if self.cells.is_empty() {
            return Vec::new();
        }

        let mut lines: Vec<Line<'static>> = Vec::new();

        // Count completed vs error
        let completed_count = self.cells.iter().filter(|c| c.status() == SubAgentStatus::Completed).count();
        let error_count = self.cells.iter().filter(|c| c.status() == SubAgentStatus::Error).count();
        let total_count = self.cells.len();

        // Group header: "● 4 agents completed (ctrl+o to expand)"
        let bullet = if error_count > 0 {
            "●".red().bold()
        } else {
            "●".green().bold()
        };

        let hint = if show_expanded {
            "(ctrl+o to collapse)"
        } else {
            "(ctrl+o to expand)"
        };

        let status_text = if error_count > 0 {
            format!("{completed_count} completed, {error_count} failed")
        } else {
            "completed".to_string()
        };

        let header_text = format!(
            "{} {} {} {}",
            total_count,
            if total_count == 1 { "agent" } else { "agents" },
            status_text,
            hint
        );

        lines.push(Line::from(vec![bullet, " ".into(), header_text.into()]));

        // Render each agent with tree prefixes
        let agent_count = self.cells.len();
        for (i, agent) in self.cells.iter().enumerate() {
            let is_last = i == agent_count - 1;
            self.render_agent_in_group(&mut lines, agent, is_last, width, show_expanded);
        }

        lines
    }

    /// Render a single agent within the group with tree prefixes.
    fn render_agent_in_group(
        &self,
        lines: &mut Vec<Line<'static>>,
        agent: &SubAgentCell,
        is_last: bool,
        width: u16,
        show_expanded: bool,
    ) {
        // Tree prefix for agent header
        let tree_prefix = if is_last { "└─ " } else { "├─ " };
        let continuation = if is_last { "   " } else { "│  " };

        // Agent bullet based on status
        let agent_bullet = match agent.status() {
            SubAgentStatus::Completed => "●".green().bold(),
            SubAgentStatus::Error => "●".red().bold(),
            SubAgentStatus::Running => "●".cyan().bold(),
        };

        // Agent header: "agent-type (description) · N tool uses · Xk tokens"
        let desc = truncate_description(agent.description(), (width as usize).saturating_sub(40));
        let tokens = agent.format_tokens();
        let header = format!(
            "{} ({}) · {} tool uses · {}",
            agent.agent_type(),
            desc,
            agent.tool_uses_count(),
            tokens,
        );

        lines.push(Line::from(vec![
            tree_prefix.dim(),
            agent_bullet,
            " ".into(),
            header.into(),
        ]));

        // Status line under agent - show the last activity
        let status_prefix = format!("{continuation}└ ");
        let status_text = agent.current_status_text();
        lines.push(Line::from(vec![status_prefix.dim(), status_text.dim()]));

        // If expanded, show forwarded events
        if show_expanded {
            let events = agent.forwarded_events();
            if !events.is_empty() {
                for (j, event) in events.iter().enumerate() {
                    let is_last_event = j == events.len() - 1;
                    let event_prefix = format!(
                        "{continuation}   {} ",
                        if is_last_event { "└" } else { "├" }
                    );

                    let status_icon = match event.status.as_str() {
                        "completed" => "✓".green(),
                        "error" => "✗".red(),
                        _ => "○".cyan(),
                    };

                    let title_text = event.title.clone().unwrap_or_default();
                    lines.push(Line::from(vec![
                        event_prefix.dim(),
                        status_icon,
                        " ".into(),
                        event.tool_name.clone().into(),
                        " ".into(),
                        title_text.dim(),
                    ]));
                }
            }
        }
    }

    /// Calculate height with explicit expanded flag.
    fn calculate_height(&self, show_expanded: bool) -> u16 {
        if self.cells.is_empty() {
            return 0;
        }

        let mut height: u16 = 1; // Group header

        for agent in &self.cells {
            height += 2; // Agent header + status line
            if show_expanded && !agent.forwarded_events().is_empty() {
                height += agent.forwarded_events().len() as u16;
            }
        }

        height
    }
}

impl HistoryCell for SubAgentGroupCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.render_with_expanded(width, self.expanded)
    }

    fn display_lines_verbose(&self, width: u16, verbose: bool) -> Vec<Line<'static>> {
        self.render_with_expanded(width, verbose || self.expanded)
    }

    fn desired_height(&self, _width: u16) -> u16 {
        self.calculate_height(self.expanded)
    }
}

/// Truncate a description string to fit within the given width.
fn truncate_description(desc: &str, max_len: usize) -> String {
    if desc.len() > max_len && max_len > 3 {
        format!("{}...", &desc[..max_len.saturating_sub(3)])
    } else {
        desc.to_string()
    }
}

fn format_mcp_invocation<'a>(invocation: McpInvocation) -> Line<'a> {
    let args_str = invocation
        .arguments
        .as_ref()
        .map(|v: &serde_json::Value| {
            // Use compact form to keep things short but readable.
            serde_json::to_string(v).unwrap_or_else(|_| v.to_string())
        })
        .unwrap_or_default();

    let invocation_spans = vec![
        invocation.server.clone().cyan(),
        ".".into(),
        invocation.tool.cyan(),
        "(".into(),
        args_str.dim(),
        ")".into(),
    ];
    invocation_spans.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec_cell::CommandOutput;
    use crate::exec_cell::ExecCall;
    use crate::exec_cell::ExecCell;
    use codex_core::config::Config;
    use codex_core::config::ConfigBuilder;
    use codex_core::config::types::McpServerConfig;
    use codex_core::config::types::McpServerTransportConfig;
    use codex_core::models_manager::manager::ModelsManager;
    use codex_core::protocol::McpAuthStatus;
    use codex_protocol::parse_command::ParsedCommand;
    use dirs::home_dir;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::collections::HashMap;

    use codex_core::protocol::ExecCommandSource;
    use mcp_types::CallToolResult;
    use mcp_types::ContentBlock;
    use mcp_types::TextContent;
    use mcp_types::Tool;
    use mcp_types::ToolInputSchema;
    async fn test_config() -> Config {
        let codex_home = std::env::temp_dir();
        ConfigBuilder::default()
            .codex_home(codex_home.clone())
            .build()
            .await
            .expect("config")
    }

    fn render_lines(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    fn render_transcript(cell: &dyn HistoryCell) -> Vec<String> {
        render_lines(&cell.transcript_lines(u16::MAX))
    }

    #[tokio::test]
    async fn mcp_tools_output_masks_sensitive_values() {
        let mut config = test_config().await;
        let mut env = HashMap::new();
        env.insert("TOKEN".to_string(), "secret".to_string());
        let stdio_config = McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command: "docs-server".to_string(),
                args: vec![],
                env: Some(env),
                env_vars: vec!["APP_TOKEN".to_string()],
                cwd: None,
            },
            enabled: true,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            enabled_tools: None,
            disabled_tools: None,
        };
        config.mcp_servers.insert("docs".to_string(), stdio_config);

        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer secret".to_string());
        let mut env_headers = HashMap::new();
        env_headers.insert("X-API-Key".to_string(), "API_KEY_ENV".to_string());
        let http_config = McpServerConfig {
            transport: McpServerTransportConfig::StreamableHttp {
                url: "https://example.com/mcp".to_string(),
                bearer_token_env_var: Some("MCP_TOKEN".to_string()),
                http_headers: Some(headers),
                env_http_headers: Some(env_headers),
            },
            enabled: true,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            enabled_tools: None,
            disabled_tools: None,
        };
        config.mcp_servers.insert("http".to_string(), http_config);

        let mut tools: HashMap<String, Tool> = HashMap::new();
        tools.insert(
            "mcp__docs__list".to_string(),
            Tool {
                annotations: None,
                description: None,
                input_schema: ToolInputSchema {
                    properties: None,
                    required: None,
                    r#type: "object".to_string(),
                },
                name: "list".to_string(),
                output_schema: None,
                title: None,
            },
        );
        tools.insert(
            "mcp__http__ping".to_string(),
            Tool {
                annotations: None,
                description: None,
                input_schema: ToolInputSchema {
                    properties: None,
                    required: None,
                    r#type: "object".to_string(),
                },
                name: "ping".to_string(),
                output_schema: None,
                title: None,
            },
        );

        let auth_statuses: HashMap<String, McpAuthStatus> = HashMap::new();
        let cell = new_mcp_tools_output(
            &config,
            tools,
            HashMap::new(),
            HashMap::new(),
            &auth_statuses,
        );
        let rendered = render_lines(&cell.display_lines(120)).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn empty_agent_message_cell_transcript() {
        let cell = AgentMessageCell::new(vec![Line::default()], false);
        assert_eq!(cell.transcript_lines(80), vec![Line::from("  ")]);
        assert_eq!(cell.desired_transcript_height(80), 1);
    }

    #[test]
    fn prefixed_wrapped_history_cell_indents_wrapped_lines() {
        let summary = Line::from(vec![
            "You ".into(),
            "approved".bold(),
            " codex to run ".into(),
            "echo something really long to ensure wrapping happens".dim(),
            " this time".bold(),
        ]);
        let cell = PrefixedWrappedHistoryCell::new(summary, "✔ ".green(), "  ");
        let rendered = render_lines(&cell.display_lines(24));
        assert_eq!(
            rendered,
            vec![
                "✔ You approved codex to".to_string(),
                "  run echo something".to_string(),
                "  really long to ensure".to_string(),
                "  wrapping happens this".to_string(),
                "  time".to_string(),
            ]
        );
    }

    #[test]
    fn web_search_history_cell_snapshot() {
        let cell = new_web_search_call(
            "example search query with several generic words to exercise wrapping".to_string(),
        );
        let rendered = render_lines(&cell.display_lines(64)).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn web_search_history_cell_wraps_with_indented_continuation() {
        let cell = new_web_search_call(
            "example search query with several generic words to exercise wrapping".to_string(),
        );
        let rendered = render_lines(&cell.display_lines(64));

        assert_eq!(
            rendered,
            vec![
                "• Searched example search query with several generic words to".to_string(),
                "  exercise wrapping".to_string(),
            ]
        );
    }

    #[test]
    fn web_search_history_cell_short_query_does_not_wrap() {
        let cell = new_web_search_call("short query".to_string());
        let rendered = render_lines(&cell.display_lines(64));

        assert_eq!(rendered, vec!["• Searched short query".to_string()]);
    }

    #[test]
    fn web_search_history_cell_transcript_snapshot() {
        let cell = new_web_search_call(
            "example search query with several generic words to exercise wrapping".to_string(),
        );
        let rendered = render_lines(&cell.transcript_lines(64)).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn active_mcp_tool_call_snapshot() {
        let invocation = McpInvocation {
            server: "search".into(),
            tool: "find_docs".into(),
            arguments: Some(json!({
                "query": "ratatui styling",
                "limit": 3,
            })),
        };

        let cell = new_active_mcp_tool_call("call-1".into(), invocation, true);
        let rendered = render_lines(&cell.display_lines(80)).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn completed_mcp_tool_call_success_snapshot() {
        let invocation = McpInvocation {
            server: "search".into(),
            tool: "find_docs".into(),
            arguments: Some(json!({
                "query": "ratatui styling",
                "limit": 3,
            })),
        };

        let result = CallToolResult {
            content: vec![ContentBlock::TextContent(TextContent {
                annotations: None,
                text: "Found styling guidance in styles.md".into(),
                r#type: "text".into(),
            })],
            is_error: None,
            structured_content: None,
        };

        let mut cell = new_active_mcp_tool_call("call-2".into(), invocation, true);
        assert!(
            cell.complete(Duration::from_millis(1420), Ok(result))
                .is_none()
        );

        let rendered = render_lines(&cell.display_lines(80)).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn completed_mcp_tool_call_error_snapshot() {
        let invocation = McpInvocation {
            server: "search".into(),
            tool: "find_docs".into(),
            arguments: Some(json!({
                "query": "ratatui styling",
                "limit": 3,
            })),
        };

        let mut cell = new_active_mcp_tool_call("call-3".into(), invocation, true);
        assert!(
            cell.complete(Duration::from_secs(2), Err("network timeout".into()))
                .is_none()
        );

        let rendered = render_lines(&cell.display_lines(80)).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn completed_mcp_tool_call_multiple_outputs_snapshot() {
        let invocation = McpInvocation {
            server: "search".into(),
            tool: "find_docs".into(),
            arguments: Some(json!({
                "query": "ratatui styling",
                "limit": 3,
            })),
        };

        let result = CallToolResult {
            content: vec![
                ContentBlock::TextContent(TextContent {
                    annotations: None,
                    text: "Found styling guidance in styles.md and additional notes in CONTRIBUTING.md.".into(),
                    r#type: "text".into(),
                }),
                ContentBlock::ResourceLink(ResourceLink {
                    annotations: None,
                    description: Some("Link to styles documentation".into()),
                    mime_type: None,
                    name: "styles.md".into(),
                    size: None,
                    title: Some("Styles".into()),
                    r#type: "resource_link".into(),
                    uri: "file:///docs/styles.md".into(),
                }),
            ],
            is_error: None,
            structured_content: None,
        };

        let mut cell = new_active_mcp_tool_call("call-4".into(), invocation, true);
        assert!(
            cell.complete(Duration::from_millis(640), Ok(result))
                .is_none()
        );

        let rendered = render_lines(&cell.display_lines(48)).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn completed_mcp_tool_call_wrapped_outputs_snapshot() {
        let invocation = McpInvocation {
            server: "metrics".into(),
            tool: "get_nearby_metric".into(),
            arguments: Some(json!({
                "query": "very_long_query_that_needs_wrapping_to_display_properly_in_the_history",
                "limit": 1,
            })),
        };

        let result = CallToolResult {
            content: vec![ContentBlock::TextContent(TextContent {
                annotations: None,
                text: "Line one of the response, which is quite long and needs wrapping.\nLine two continues the response with more detail.".into(),
                r#type: "text".into(),
            })],
            is_error: None,
            structured_content: None,
        };

        let mut cell = new_active_mcp_tool_call("call-5".into(), invocation, true);
        assert!(
            cell.complete(Duration::from_millis(1280), Ok(result))
                .is_none()
        );

        let rendered = render_lines(&cell.display_lines(40)).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn completed_mcp_tool_call_multiple_outputs_inline_snapshot() {
        let invocation = McpInvocation {
            server: "metrics".into(),
            tool: "summary".into(),
            arguments: Some(json!({
                "metric": "trace.latency",
                "window": "15m",
            })),
        };

        let result = CallToolResult {
            content: vec![
                ContentBlock::TextContent(TextContent {
                    annotations: None,
                    text: "Latency summary: p50=120ms, p95=480ms.".into(),
                    r#type: "text".into(),
                }),
                ContentBlock::TextContent(TextContent {
                    annotations: None,
                    text: "No anomalies detected.".into(),
                    r#type: "text".into(),
                }),
            ],
            is_error: None,
            structured_content: None,
        };

        let mut cell = new_active_mcp_tool_call("call-6".into(), invocation, true);
        assert!(
            cell.complete(Duration::from_millis(320), Ok(result))
                .is_none()
        );

        let rendered = render_lines(&cell.display_lines(120)).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn session_header_includes_reasoning_level_when_present() {
        let cell = SessionHeaderHistoryCell::new(
            "gpt-4o".to_string(),
            Some(ReasoningEffortConfig::High),
            std::env::temp_dir(),
            "test",
        );

        let lines = render_lines(&cell.display_lines(80));
        let model_line = lines
            .into_iter()
            .find(|line| line.contains("model:"))
            .expect("model line");

        assert!(model_line.contains("gpt-4o high"));
        assert!(model_line.contains("/model to change"));
    }

    #[test]
    fn session_header_directory_center_truncates() {
        let mut dir = home_dir().expect("home directory");
        for part in ["hello", "the", "fox", "is", "very", "fast"] {
            dir.push(part);
        }

        let formatted = SessionHeaderHistoryCell::format_directory_inner(&dir, Some(24));
        let sep = std::path::MAIN_SEPARATOR;
        let expected = format!("~{sep}hello{sep}the{sep}…{sep}very{sep}fast");
        assert_eq!(formatted, expected);
    }

    #[test]
    fn session_header_directory_front_truncates_long_segment() {
        let mut dir = home_dir().expect("home directory");
        dir.push("supercalifragilisticexpialidocious");

        let formatted = SessionHeaderHistoryCell::format_directory_inner(&dir, Some(18));
        let sep = std::path::MAIN_SEPARATOR;
        let expected = format!("~{sep}…cexpialidocious");
        assert_eq!(formatted, expected);
    }

    #[test]
    fn coalesces_sequential_reads_within_one_call() {
        // Build one exec cell with a Search followed by two Reads
        let call_id = "c1".to_string();
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: call_id.clone(),
                command: vec!["bash".into(), "-lc".into(), "echo".into()],
                parsed: vec![
                    ParsedCommand::Search {
                        query: Some("shimmer_spans".into()),
                        path: None,
                        cmd: "rg shimmer_spans".into(),
                    },
                    ParsedCommand::Read {
                        name: "shimmer.rs".into(),
                        cmd: "cat shimmer.rs".into(),
                        path: "shimmer.rs".into(),
                    },
                    ParsedCommand::Read {
                        name: "status_indicator_widget.rs".into(),
                        cmd: "cat status_indicator_widget.rs".into(),
                        path: "status_indicator_widget.rs".into(),
                    },
                ],
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );
        // Mark call complete so markers are ✓
        cell.complete_call(&call_id, CommandOutput::default(), Duration::from_millis(1));

        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines).join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn coalesces_reads_across_multiple_calls() {
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: "c1".to_string(),
                command: vec!["bash".into(), "-lc".into(), "echo".into()],
                parsed: vec![ParsedCommand::Search {
                    query: Some("shimmer_spans".into()),
                    path: None,
                    cmd: "rg shimmer_spans".into(),
                }],
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );
        // Call 1: Search only
        cell.complete_call("c1", CommandOutput::default(), Duration::from_millis(1));
        // Call 2: Read A
        cell = cell
            .with_added_call(
                "c2".into(),
                vec!["bash".into(), "-lc".into(), "echo".into()],
                vec![ParsedCommand::Read {
                    name: "shimmer.rs".into(),
                    cmd: "cat shimmer.rs".into(),
                    path: "shimmer.rs".into(),
                }],
                ExecCommandSource::Agent,
                None,
            )
            .unwrap();
        cell.complete_call("c2", CommandOutput::default(), Duration::from_millis(1));
        // Call 3: Read B
        cell = cell
            .with_added_call(
                "c3".into(),
                vec!["bash".into(), "-lc".into(), "echo".into()],
                vec![ParsedCommand::Read {
                    name: "status_indicator_widget.rs".into(),
                    cmd: "cat status_indicator_widget.rs".into(),
                    path: "status_indicator_widget.rs".into(),
                }],
                ExecCommandSource::Agent,
                None,
            )
            .unwrap();
        cell.complete_call("c3", CommandOutput::default(), Duration::from_millis(1));

        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines).join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn coalesced_reads_dedupe_names() {
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: "c1".to_string(),
                command: vec!["bash".into(), "-lc".into(), "echo".into()],
                parsed: vec![
                    ParsedCommand::Read {
                        name: "auth.rs".into(),
                        cmd: "cat auth.rs".into(),
                        path: "auth.rs".into(),
                    },
                    ParsedCommand::Read {
                        name: "auth.rs".into(),
                        cmd: "cat auth.rs".into(),
                        path: "auth.rs".into(),
                    },
                    ParsedCommand::Read {
                        name: "shimmer.rs".into(),
                        cmd: "cat shimmer.rs".into(),
                        path: "shimmer.rs".into(),
                    },
                ],
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );
        cell.complete_call("c1", CommandOutput::default(), Duration::from_millis(1));
        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines).join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn multiline_command_wraps_with_extra_indent_on_subsequent_lines() {
        // Create a completed exec cell with a multiline command
        let cmd = "set -o pipefail\ncargo test --all-features --quiet".to_string();
        let call_id = "c1".to_string();
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: call_id.clone(),
                command: vec!["bash".into(), "-lc".into(), cmd],
                parsed: Vec::new(),
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );
        // Mark call complete so it renders as "Ran"
        cell.complete_call(&call_id, CommandOutput::default(), Duration::from_millis(1));

        // Small width to force wrapping on both lines
        let width: u16 = 28;
        let lines = cell.display_lines(width);
        let rendered = render_lines(&lines).join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn single_line_command_compact_when_fits() {
        let call_id = "c1".to_string();
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: call_id.clone(),
                command: vec!["echo".into(), "ok".into()],
                parsed: Vec::new(),
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );
        cell.complete_call(&call_id, CommandOutput::default(), Duration::from_millis(1));
        // Wide enough that it fits inline
        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines).join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn single_line_command_wraps_with_four_space_continuation() {
        let call_id = "c1".to_string();
        let long = "a_very_long_token_without_spaces_to_force_wrapping".to_string();
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: call_id.clone(),
                command: vec!["bash".into(), "-lc".into(), long],
                parsed: Vec::new(),
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );
        cell.complete_call(&call_id, CommandOutput::default(), Duration::from_millis(1));
        let lines = cell.display_lines(24);
        let rendered = render_lines(&lines).join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn multiline_command_without_wrap_uses_branch_then_eight_spaces() {
        let call_id = "c1".to_string();
        let cmd = "echo one\necho two".to_string();
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: call_id.clone(),
                command: vec!["bash".into(), "-lc".into(), cmd],
                parsed: Vec::new(),
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );
        cell.complete_call(&call_id, CommandOutput::default(), Duration::from_millis(1));
        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines).join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn multiline_command_both_lines_wrap_with_correct_prefixes() {
        let call_id = "c1".to_string();
        let cmd = "first_token_is_long_enough_to_wrap\nsecond_token_is_also_long_enough_to_wrap"
            .to_string();
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: call_id.clone(),
                command: vec!["bash".into(), "-lc".into(), cmd],
                parsed: Vec::new(),
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );
        cell.complete_call(&call_id, CommandOutput::default(), Duration::from_millis(1));
        let lines = cell.display_lines(28);
        let rendered = render_lines(&lines).join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn stderr_tail_more_than_five_lines_snapshot() {
        // Build an exec cell with a non-zero exit and 10 lines on stderr to exercise
        // the head/tail rendering and gutter prefixes.
        let call_id = "c_err".to_string();
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: call_id.clone(),
                command: vec!["bash".into(), "-lc".into(), "seq 1 10 1>&2 && false".into()],
                parsed: Vec::new(),
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );
        let stderr: String = (1..=10)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        cell.complete_call(
            &call_id,
            CommandOutput {
                exit_code: 1,
                formatted_output: String::new(),
                aggregated_output: stderr,
            },
            Duration::from_millis(1),
        );

        let rendered = cell
            .display_lines(80)
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn ran_cell_multiline_with_stderr_snapshot() {
        // Build an exec cell that completes (so it renders as "Ran") with a
        // command long enough that it must render on its own line under the
        // header, and include a couple of stderr lines to verify the output
        // block prefixes and wrapping.
        let call_id = "c_wrap_err".to_string();
        let long_cmd =
            "echo this_is_a_very_long_single_token_that_will_wrap_across_the_available_width";
        let mut cell = ExecCell::new(
            ExecCall {
                call_id: call_id.clone(),
                command: vec!["bash".into(), "-lc".into(), long_cmd.to_string()],
                parsed: Vec::new(),
                output: None,
                source: ExecCommandSource::Agent,
                start_time: Some(Instant::now()),
                duration: None,
                interaction_input: None,
            },
            true,
        );

        let stderr = "error: first line on stderr\nerror: second line on stderr".to_string();
        cell.complete_call(
            &call_id,
            CommandOutput {
                exit_code: 1,
                formatted_output: String::new(),
                aggregated_output: stderr,
            },
            Duration::from_millis(5),
        );

        // Narrow width to force the command to render under the header line.
        let width: u16 = 28;
        let rendered = cell
            .display_lines(width)
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        insta::assert_snapshot!(rendered);
    }
    #[test]
    fn user_history_cell_wraps_and_prefixes_each_line_snapshot() {
        let msg = "one two three four five six seven";
        let cell = UserHistoryCell {
            message: msg.to_string(),
        };

        // Small width to force wrapping more clearly. Effective wrap width is width-2 due to the ▌ prefix and trailing space.
        let width: u16 = 12;
        let lines = cell.display_lines(width);
        let rendered = render_lines(&lines).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn reasoning_summary_block() {
        let reasoning_format = ReasoningSummaryFormat::Experimental;
        let cell = new_reasoning_summary_block(
            "**High level reasoning**\n\nDetailed reasoning goes here.".to_string(),
            reasoning_format,
        );

        let rendered_display = render_lines(&cell.display_lines(80));
        assert_eq!(rendered_display, vec!["• Detailed reasoning goes here."]);

        let rendered_transcript = render_transcript(cell.as_ref());
        assert_eq!(rendered_transcript, vec!["• Detailed reasoning goes here."]);
    }

    #[test]
    fn reasoning_summary_block_returns_reasoning_cell_when_feature_disabled() {
        let reasoning_format = ReasoningSummaryFormat::Experimental;
        let cell = new_reasoning_summary_block(
            "Detailed reasoning goes here.".to_string(),
            reasoning_format,
        );

        let rendered = render_transcript(cell.as_ref());
        assert_eq!(rendered, vec!["• Detailed reasoning goes here."]);
    }

    #[tokio::test]
    async fn reasoning_summary_block_respects_config_overrides() {
        let mut config = test_config().await;
        config.model = Some("gpt-3.5-turbo".to_string());
        config.model_supports_reasoning_summaries = Some(true);
        config.model_reasoning_summary_format = Some(ReasoningSummaryFormat::Experimental);
        let model_family =
            ModelsManager::construct_model_family_offline(&config.model.clone().unwrap(), &config);
        assert_eq!(
            model_family.reasoning_summary_format,
            ReasoningSummaryFormat::Experimental
        );

        let cell = new_reasoning_summary_block(
            "**High level reasoning**\n\nDetailed reasoning goes here.".to_string(),
            model_family.reasoning_summary_format,
        );

        let rendered_display = render_lines(&cell.display_lines(80));
        assert_eq!(rendered_display, vec!["• Detailed reasoning goes here."]);
    }

    #[test]
    fn reasoning_summary_block_falls_back_when_header_is_missing() {
        let reasoning_format = ReasoningSummaryFormat::Experimental;
        let cell = new_reasoning_summary_block(
            "**High level reasoning without closing".to_string(),
            reasoning_format,
        );

        let rendered = render_transcript(cell.as_ref());
        assert_eq!(rendered, vec!["• **High level reasoning without closing"]);
    }

    #[test]
    fn reasoning_summary_block_falls_back_when_summary_is_missing() {
        let reasoning_format = ReasoningSummaryFormat::Experimental;
        let cell = new_reasoning_summary_block(
            "**High level reasoning without closing**".to_string(),
            reasoning_format.clone(),
        );

        let rendered = render_transcript(cell.as_ref());
        assert_eq!(rendered, vec!["• High level reasoning without closing"]);

        let cell = new_reasoning_summary_block(
            "**High level reasoning without closing**\n\n  ".to_string(),
            reasoning_format,
        );

        let rendered = render_transcript(cell.as_ref());
        assert_eq!(rendered, vec!["• High level reasoning without closing"]);
    }

    #[test]
    fn reasoning_summary_block_splits_header_and_summary_when_present() {
        let reasoning_format = ReasoningSummaryFormat::Experimental;
        let cell = new_reasoning_summary_block(
            "**High level plan**\n\nWe should fix the bug next.".to_string(),
            reasoning_format,
        );

        let rendered_display = render_lines(&cell.display_lines(80));
        assert_eq!(rendered_display, vec!["• We should fix the bug next."]);

        let rendered_transcript = render_transcript(cell.as_ref());
        assert_eq!(rendered_transcript, vec!["• We should fix the bug next."]);
    }

    #[test]
    fn deprecation_notice_renders_summary_with_details() {
        let cell = new_deprecation_notice(
            "Feature flag `foo`".to_string(),
            Some("Use flag `bar` instead.".to_string()),
        );
        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines);
        assert_eq!(
            rendered,
            vec![
                "⚠ Feature flag `foo`".to_string(),
                "Use flag `bar` instead.".to_string(),
            ]
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // SubAgentCell Tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn subagent_cell_renders_running_state() {
        use codex_core::protocol::SubAgentBeginEvent;

        let begin_event = SubAgentBeginEvent {
            call_id: "call-123".to_string(),
            session_id: "task-456".to_string(),
            agent_type: "explore".to_string(),
            description: "Search for rust files".to_string(),
            resumed: false,
        };

        let cell = new_subagent_cell(begin_event, false);
        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines);

        // Should have header and description
        assert!(rendered.len() >= 2);
        assert!(rendered[0].contains("explore"));
        assert!(rendered[0].contains("Running"));
        assert!(rendered[0].contains("0 tool uses"));
        assert!(rendered[1].contains("Search for rust files"));
    }

    #[test]
    fn subagent_cell_renders_resumed_state() {
        use codex_core::protocol::SubAgentBeginEvent;

        let begin_event = SubAgentBeginEvent {
            call_id: "call-123".to_string(),
            session_id: "task-456".to_string(),
            agent_type: "explore".to_string(),
            description: "Resuming previous search".to_string(),
            resumed: true,
        };

        let cell = new_subagent_cell(begin_event, false);
        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines);

        // Should show "Resumed" instead of "Running"
        assert!(rendered.len() >= 2);
        assert!(rendered[0].contains("explore"));
        assert!(rendered[0].contains("Resumed"), "Expected 'Resumed' in: {}", rendered[0]);
        assert!(!rendered[0].contains("Running"), "Should not contain 'Running' when resumed");
        assert!(rendered[1].contains("Resuming previous search"));
    }

    #[test]
    fn subagent_cell_renders_completed_state() {
        use codex_core::protocol::{SubAgentBeginEvent, SubAgentEndEvent, SubAgentTokenUsage};

        let begin_event = SubAgentBeginEvent {
            call_id: "call-123".to_string(),
            session_id: "task-456".to_string(),
            agent_type: "explore".to_string(),
            description: "Search for rust files".to_string(),
            resumed: false,
        };

        let mut cell = new_subagent_cell(begin_event, false);

        let end_event = SubAgentEndEvent {
            call_id: "call-123".to_string(),
            session_id: "task-456".to_string(),
            success: true,
            output: "Found 10 files".to_string(),
            duration_ms: 1500,
            tool_summary: vec![],
            token_usage: Some(SubAgentTokenUsage {
                input_tokens: 500,
                output_tokens: 200,
                total_tokens: 700,
            }),
        };

        cell.complete(&end_event);
        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines);

        assert!(rendered[0].contains("Done"));
        assert!(rendered[0].contains("700 tokens"));
    }

    #[test]
    fn subagent_cell_formats_tokens_correctly() {
        use codex_core::protocol::{SubAgentBeginEvent, SubAgentEndEvent, SubAgentTokenUsage};

        let begin_event = SubAgentBeginEvent {
            call_id: "call-123".to_string(),
            session_id: "task-456".to_string(),
            agent_type: "explore".to_string(),
            description: "Test".to_string(),
            resumed: false,
        };

        let mut cell = new_subagent_cell(begin_event, false);

        // Test with 101500 tokens (should format as "101.5k tokens")
        let end_event = SubAgentEndEvent {
            call_id: "call-123".to_string(),
            session_id: "task-456".to_string(),
            success: true,
            output: "Done".to_string(),
            duration_ms: 1000,
            tool_summary: vec![],
            token_usage: Some(SubAgentTokenUsage {
                input_tokens: 80000,
                output_tokens: 21500,
                total_tokens: 101500,
            }),
        };

        cell.complete(&end_event);
        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines);

        assert!(rendered[0].contains("101.5k tokens"), "Expected '101.5k tokens' in: {}", rendered[0]);
    }

    #[test]
    fn subagent_cell_toggle_expanded() {
        use codex_core::protocol::SubAgentBeginEvent;

        let begin_event = SubAgentBeginEvent {
            call_id: "call-123".to_string(),
            session_id: "task-456".to_string(),
            agent_type: "explore".to_string(),
            description: "Test".to_string(),
            resumed: false,
        };

        let mut cell = new_subagent_cell(begin_event, false);

        // Add a forwarded event
        cell.add_forwarded_event(ForwardedToolEvent {
            tool_name: "Read".to_string(),
            title: Some("file.rs".to_string()),
            status: "completed".to_string(),
        });

        // Initially collapsed
        let lines_collapsed = cell.display_lines(80);
        let rendered_collapsed = render_lines(&lines_collapsed);
        assert!(!rendered_collapsed.iter().any(|l| l.contains("Read")), "Tool should not be visible when collapsed");

        // Toggle to expanded
        cell.toggle_expanded();
        let lines_expanded = cell.display_lines(80);
        let rendered_expanded = render_lines(&lines_expanded);
        assert!(rendered_expanded.iter().any(|l| l.contains("Read")), "Tool should be visible when expanded");
        assert!(rendered_expanded.iter().any(|l| l.contains("file.rs")));
    }

    #[test]
    fn subagent_cell_error_state() {
        use codex_core::protocol::{SubAgentBeginEvent, SubAgentEndEvent};

        let begin_event = SubAgentBeginEvent {
            call_id: "call-123".to_string(),
            session_id: "task-456".to_string(),
            agent_type: "explore".to_string(),
            description: "Test".to_string(),
            resumed: false,
        };

        let mut cell = new_subagent_cell(begin_event, false);

        let end_event = SubAgentEndEvent {
            call_id: "call-123".to_string(),
            session_id: "task-456".to_string(),
            success: false,
            output: "Task failed".to_string(),
            duration_ms: 500,
            tool_summary: vec![],
            token_usage: None,
        };

        cell.complete(&end_event);
        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines);

        assert!(rendered[0].contains("Error"));
        assert!(rendered[0].contains("-- tokens")); // No token usage
    }

    // ─────────────────────────────────────────────────────────────────────────
    // RunningAgentsGroup Tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn running_agents_group_renders_single_agent() {
        use codex_core::protocol::SubAgentBeginEvent;

        let begin_event = SubAgentBeginEvent {
            call_id: "call-1".to_string(),
            session_id: "sess-1".to_string(),
            agent_type: "explore".to_string(),
            description: "Search for files".to_string(),
            resumed: false,
        };

        let cell = new_subagent_cell(begin_event, false);
        let group = RunningAgentsGroup::new(vec![&cell], false, false);

        let lines = group.render_lines(80);
        let rendered = render_lines(&lines);

        // Should have: header, agent header, agent status
        assert!(rendered.len() >= 3, "Expected at least 3 lines, got {}", rendered.len());
        assert!(rendered[0].contains("Running 1 agent"), "Header should show '1 agent': {}", rendered[0]);
        assert!(rendered[1].contains("explore"), "Should contain agent type: {}", rendered[1]);
        assert!(rendered[2].contains("Initializing"), "Should show initializing status: {}", rendered[2]);
    }

    #[test]
    fn running_agents_group_renders_multiple_agents_with_tree() {
        use codex_core::protocol::SubAgentBeginEvent;

        let begin1 = SubAgentBeginEvent {
            call_id: "call-1".to_string(),
            session_id: "sess-1".to_string(),
            agent_type: "explore".to_string(),
            description: "First task".to_string(),
            resumed: false,
        };
        let begin2 = SubAgentBeginEvent {
            call_id: "call-2".to_string(),
            session_id: "sess-2".to_string(),
            agent_type: "analyze".to_string(),
            description: "Second task".to_string(),
            resumed: false,
        };

        let cell1 = new_subagent_cell(begin1, false);
        let cell2 = new_subagent_cell(begin2, false);
        let group = RunningAgentsGroup::new(vec![&cell1, &cell2], false, false);

        let lines = group.render_lines(80);
        let rendered = render_lines(&lines);

        // Should have: header, 2x(agent header + status)
        assert!(rendered.len() >= 5, "Expected at least 5 lines, got {}", rendered.len());
        assert!(rendered[0].contains("Running 2 agents"), "Header should show '2 agents': {}", rendered[0]);

        // First agent uses ├─, last uses └─
        assert!(rendered[1].contains("├"), "First agent should have ├ prefix: {}", rendered[1]);
        assert!(rendered[3].contains("└"), "Last agent should have └ prefix: {}", rendered[3]);
    }

    #[test]
    fn running_agents_group_expanded_shows_events() {
        use codex_core::protocol::SubAgentBeginEvent;

        let begin = SubAgentBeginEvent {
            call_id: "call-1".to_string(),
            session_id: "sess-1".to_string(),
            agent_type: "explore".to_string(),
            description: "Task".to_string(),
            resumed: false,
        };

        let mut cell = new_subagent_cell(begin, false);
        cell.add_forwarded_event(ForwardedToolEvent {
            tool_name: "shell".to_string(),
            title: Some("ls -la".to_string()),
            status: "completed".to_string(),
        });

        let group = RunningAgentsGroup::new(vec![&cell], true, false);
        let lines = group.render_lines(80);
        let rendered = render_lines(&lines);

        // Should include the forwarded event line when expanded
        assert!(
            rendered.iter().any(|l| l.contains("shell")),
            "Should contain tool name 'shell' when expanded: {:?}",
            rendered
        );
    }

    #[test]
    fn running_agents_group_collapsed_hides_event_details() {
        use codex_core::protocol::SubAgentBeginEvent;

        let begin = SubAgentBeginEvent {
            call_id: "call-1".to_string(),
            session_id: "sess-1".to_string(),
            agent_type: "explore".to_string(),
            description: "Task".to_string(),
            resumed: false,
        };

        let mut cell = new_subagent_cell(begin, false);
        cell.add_forwarded_event(ForwardedToolEvent {
            tool_name: "shell".to_string(),
            title: Some("ls -la".to_string()),
            status: "completed".to_string(),
        });

        let group = RunningAgentsGroup::new(vec![&cell], false, false);
        let lines = group.render_lines(80);
        let rendered = render_lines(&lines);

        // When collapsed, should NOT include the full tool event with its title (ls -la)
        // The status line "Completed shell" is still visible as the status text
        assert!(
            !rendered.iter().any(|l| l.contains("ls -la")),
            "Should NOT contain event title 'ls -la' when collapsed: {:?}",
            rendered
        );
        // Should only have header (1) + agent header (1) + status (1) = 3 lines
        assert_eq!(rendered.len(), 3, "Collapsed should have 3 lines: {:?}", rendered);
    }

    #[test]
    fn running_agents_group_height_calculation() {
        use codex_core::protocol::SubAgentBeginEvent;

        let begin1 = SubAgentBeginEvent {
            call_id: "call-1".to_string(),
            session_id: "sess-1".to_string(),
            agent_type: "explore".to_string(),
            description: "Task 1".to_string(),
            resumed: false,
        };
        let begin2 = SubAgentBeginEvent {
            call_id: "call-2".to_string(),
            session_id: "sess-2".to_string(),
            agent_type: "analyze".to_string(),
            description: "Task 2".to_string(),
            resumed: false,
        };

        let cell1 = new_subagent_cell(begin1, false);
        let cell2 = new_subagent_cell(begin2, false);

        // Collapsed: 1 (header) + 2*2 (agent + status for each) = 5
        let group_collapsed = RunningAgentsGroup::new(vec![&cell1, &cell2], false, false);
        assert_eq!(group_collapsed.calculate_height(), 5);
    }

    #[test]
    fn running_agents_group_empty() {
        let group: RunningAgentsGroup = RunningAgentsGroup::new(vec![], false, false);

        assert!(group.is_empty());
        assert_eq!(group.calculate_height(), 0);

        let lines = group.render_lines(80);
        assert!(lines.is_empty());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // SubAgentGroupCell Tests (for completed agents in history)
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn subagent_group_cell_renders_single_completed_agent() {
        use codex_core::protocol::{SubAgentBeginEvent, SubAgentEndEvent};

        let begin = SubAgentBeginEvent {
            call_id: "call-1".to_string(),
            session_id: "sess-1".to_string(),
            agent_type: "explore".to_string(),
            description: "Search files".to_string(),
            resumed: false,
        };

        let mut cell = new_subagent_cell(begin, false);
        cell.complete(&SubAgentEndEvent {
            call_id: "call-1".to_string(),
            session_id: "sess-1".to_string(),
            success: true,
            output: "Done".to_string(),
            duration_ms: 1000,
            tool_summary: vec![],
            token_usage: Some(SubAgentTokenUsage {
                input_tokens: 5000,
                output_tokens: 2000,
                total_tokens: 7000,
            }),
        });

        let group = SubAgentGroupCell::new(vec![cell]);
        let lines = group.display_lines(80);
        let rendered = render_lines(&lines);

        assert!(rendered.len() >= 3, "Expected at least 3 lines: {:?}", rendered);
        assert!(rendered[0].contains("1 agent"), "Header should show '1 agent': {}", rendered[0]);
        assert!(rendered[0].contains("completed"), "Header should show 'completed': {}", rendered[0]);
        assert!(rendered[1].contains("explore"), "Should contain agent type: {}", rendered[1]);
        assert!(rendered[1].contains("7.0k tokens"), "Should show token count: {}", rendered[1]);
    }

    #[test]
    fn subagent_group_cell_renders_multiple_completed_agents() {
        use codex_core::protocol::{SubAgentBeginEvent, SubAgentEndEvent};

        let begin1 = SubAgentBeginEvent {
            call_id: "call-1".to_string(),
            session_id: "sess-1".to_string(),
            agent_type: "explore".to_string(),
            description: "First task".to_string(),
            resumed: false,
        };
        let begin2 = SubAgentBeginEvent {
            call_id: "call-2".to_string(),
            session_id: "sess-2".to_string(),
            agent_type: "analyze".to_string(),
            description: "Second task".to_string(),
            resumed: false,
        };

        let mut cell1 = new_subagent_cell(begin1, false);
        let mut cell2 = new_subagent_cell(begin2, false);

        cell1.complete(&SubAgentEndEvent {
            call_id: "call-1".to_string(),
            session_id: "sess-1".to_string(),
            success: true,
            output: "Done".to_string(),
            duration_ms: 1000,
            tool_summary: vec![],
            token_usage: None,
        });
        cell2.complete(&SubAgentEndEvent {
            call_id: "call-2".to_string(),
            session_id: "sess-2".to_string(),
            success: true,
            output: "Done".to_string(),
            duration_ms: 2000,
            tool_summary: vec![],
            token_usage: None,
        });

        let group = SubAgentGroupCell::new(vec![cell1, cell2]);
        let lines = group.display_lines(80);
        let rendered = render_lines(&lines);

        assert!(rendered.len() >= 5, "Expected at least 5 lines: {:?}", rendered);
        assert!(rendered[0].contains("2 agents"), "Header should show '2 agents': {}", rendered[0]);
        // First agent uses ├─, last uses └─
        assert!(rendered[1].contains("├"), "First agent should have ├ prefix: {}", rendered[1]);
        assert!(rendered[3].contains("└"), "Last agent should have └ prefix: {}", rendered[3]);
    }

    #[test]
    fn subagent_group_cell_shows_error_status() {
        use codex_core::protocol::{SubAgentBeginEvent, SubAgentEndEvent};

        let begin1 = SubAgentBeginEvent {
            call_id: "call-1".to_string(),
            session_id: "sess-1".to_string(),
            agent_type: "explore".to_string(),
            description: "Success task".to_string(),
            resumed: false,
        };
        let begin2 = SubAgentBeginEvent {
            call_id: "call-2".to_string(),
            session_id: "sess-2".to_string(),
            agent_type: "analyze".to_string(),
            description: "Failed task".to_string(),
            resumed: false,
        };

        let mut cell1 = new_subagent_cell(begin1, false);
        let mut cell2 = new_subagent_cell(begin2, false);

        cell1.complete(&SubAgentEndEvent {
            call_id: "call-1".to_string(),
            session_id: "sess-1".to_string(),
            success: true,
            output: "Done".to_string(),
            duration_ms: 1000,
            tool_summary: vec![],
            token_usage: None,
        });
        cell2.complete(&SubAgentEndEvent {
            call_id: "call-2".to_string(),
            session_id: "sess-2".to_string(),
            success: false, // FAILED
            output: "Error".to_string(),
            duration_ms: 500,
            tool_summary: vec![],
            token_usage: None,
        });

        let group = SubAgentGroupCell::new(vec![cell1, cell2]);
        let lines = group.display_lines(80);
        let rendered = render_lines(&lines);

        // Header should show "1 completed, 1 failed"
        assert!(rendered[0].contains("1 completed"), "Header should show completed count: {}", rendered[0]);
        assert!(rendered[0].contains("1 failed"), "Header should show failed count: {}", rendered[0]);
    }

    #[test]
    fn subagent_group_cell_verbose_shows_events() {
        use codex_core::protocol::{SubAgentBeginEvent, SubAgentEndEvent};

        let begin = SubAgentBeginEvent {
            call_id: "call-1".to_string(),
            session_id: "sess-1".to_string(),
            agent_type: "explore".to_string(),
            description: "Task".to_string(),
            resumed: false,
        };

        let mut cell = new_subagent_cell(begin, false);
        cell.add_forwarded_event(ForwardedToolEvent {
            tool_name: "Read".to_string(),
            title: Some("config.json".to_string()),
            status: "completed".to_string(),
        });
        cell.complete(&SubAgentEndEvent {
            call_id: "call-1".to_string(),
            session_id: "sess-1".to_string(),
            success: true,
            output: "Done".to_string(),
            duration_ms: 1000,
            tool_summary: vec![],
            token_usage: None,
        });

        let group = SubAgentGroupCell::new(vec![cell]);

        // Verbose mode should show tool events
        let lines_verbose = group.display_lines_verbose(80, true);
        let rendered = render_lines(&lines_verbose);

        assert!(
            rendered.iter().any(|l| l.contains("Read")),
            "Verbose mode should show tool name 'Read': {:?}",
            rendered
        );
        assert!(
            rendered.iter().any(|l| l.contains("config.json")),
            "Verbose mode should show tool title 'config.json': {:?}",
            rendered
        );
    }
}
