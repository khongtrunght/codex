use super::model::SubAgentCell;
use super::model::SubAgentEntry;
use super::model::SubAgentStatus;
use crate::exec_cell::spinner;
use crate::exec_command::strip_bash_lc_and_escape;
use crate::history_cell::HistoryCell;
use codex_protocol::items::TurnItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::WebSearchAction;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::SubAgentBeginEvent;
use ratatui::prelude::*;
use textwrap::wrap;

/// Create a new SubAgentCell from a begin event.
pub(crate) fn new_subagent_cell(
    begin_event: SubAgentBeginEvent,
    animations_enabled: bool,
) -> SubAgentCell {
    // Store the prompt if non-empty
    let prompt = if begin_event.prompt.is_empty() {
        None
    } else {
        Some(begin_event.prompt)
    };
    let entry = SubAgentEntry::new(
        begin_event.call_id,
        begin_event.agent_type,
        begin_event.description,
        prompt,
    );
    SubAgentCell::new(entry, animations_enabled)
}

/// Status of a tool call.
#[derive(Debug, Clone)]
enum ToolCallResult {
    Running,
    Success(String), // Output text
    Error(String),   // Error message
    #[allow(dead_code)]
    Rejected(String), // For future: tool rejection handling
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolCallKind {
    ExecCommand,
    PatchApply,
    WebSearch,
    Mcp,
    ApiFunction,
    ApiCustom,
}

/// Extracted tool call info for rendering.
#[derive(Debug, Clone)]
struct ToolCallInfo {
    kind: ToolCallKind,
    tool_name: String,
    args: String, // e.g., "echo \"Hello World\"" or file path
    result: ToolCallResult,
}

/// Truncate string to n characters, adding "…" if truncated.
fn truncate_to_n_chars(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else if n > 1 {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    } else {
        "…".to_string()
    }
}

fn wrap_plain_text_lines(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut wrapped = Vec::new();
    for line in text.split('\n') {
        if line.is_empty() {
            wrapped.push(String::new());
            continue;
        }
        wrapped.extend(
            wrap(line, width)
                .into_iter()
                .map(std::borrow::Cow::into_owned),
        );
    }
    wrapped
}

fn push_wrapped_lines(lines: &mut Vec<Line<'static>>, prefix: &str, text: &str, width: u16) {
    let available = width.saturating_sub(prefix.chars().count() as u16) as usize;
    let prefix = prefix.to_string();
    for line in wrap_plain_text_lines(text, available) {
        lines.push(Line::from(vec![prefix.clone().dim(), line.into()]));
    }
}

fn push_wrapped_lines_red(lines: &mut Vec<Line<'static>>, prefix: &str, text: &str, width: u16) {
    let available = width.saturating_sub(prefix.chars().count() as u16) as usize;
    let prefix = prefix.to_string();
    for line in wrap_plain_text_lines(text, available) {
        lines.push(Line::from(vec![prefix.clone().dim(), line.red()]));
    }
}

fn push_camel_word(word: &str, out: &mut String) {
    let mut chars = word.chars();
    if let Some(first) = chars.next() {
        out.push(first.to_ascii_uppercase());
        for ch in chars {
            out.push(ch.to_ascii_lowercase());
        }
    }
}

fn to_camel_case(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    let has_separator = input.chars().any(|c| matches!(c, '_' | '-' | ' ' | '.'));
    let is_all_lower = input
        .chars()
        .all(|c| !c.is_ascii_alphabetic() || c.is_ascii_lowercase());
    if !has_separator && !is_all_lower {
        return input.to_string();
    }

    let mut out = String::new();
    let mut current = String::new();

    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch);
        } else if !current.is_empty() {
            push_camel_word(&current, &mut out);
            current.clear();
        }
    }
    if !current.is_empty() {
        push_camel_word(&current, &mut out);
    }
    out
}

fn tool_name_key(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn should_display_tool_call(call: &ToolCallInfo) -> bool {
    match call.kind {
        ToolCallKind::ExecCommand | ToolCallKind::PatchApply | ToolCallKind::WebSearch => true,
        ToolCallKind::ApiFunction | ToolCallKind::ApiCustom => {
            tool_name_key(&call.tool_name) == "readfile"
        }
        ToolCallKind::Mcp => false,
    }
}

fn display_tool_name(tool_name: &str) -> String {
    to_camel_case(tool_name)
}

fn tool_calls_header_text(total: usize, visible: usize) -> String {
    if total == visible {
        format!("Tool calls ({total})")
    } else {
        format!("Tool calls ({total} total, showing {visible})")
    }
}

fn web_search_args(action: &WebSearchAction) -> String {
    match action {
        WebSearchAction::Search { query } => match query.as_deref() {
            Some(query) if !query.is_empty() => format!("Search: {query}"),
            _ => "Search".to_string(),
        },
        WebSearchAction::OpenPage { url } => match url.as_deref() {
            Some(url) if !url.is_empty() => format!("OpenPage: {url}"),
            _ => "OpenPage".to_string(),
        },
        WebSearchAction::FindInPage { url, pattern } => {
            let url = url.as_deref().unwrap_or_default();
            let pattern = pattern.as_deref().unwrap_or_default();
            if !url.is_empty() && !pattern.is_empty() {
                format!("FindInPage: {url} · {pattern}")
            } else if !url.is_empty() {
                format!("FindInPage: {url}")
            } else if !pattern.is_empty() {
                format!("FindInPage: {pattern}")
            } else {
                "FindInPage".to_string()
            }
        }
        WebSearchAction::Other => "WebSearch".to_string(),
    }
}

fn render_tool_call_block(
    lines: &mut Vec<Line<'static>>,
    item_prefix: &str,
    detail_prefix: &str,
    call: &ToolCallInfo,
    width: u16,
) {
    let item_prefix = item_prefix.to_string();
    let detail_prefix = detail_prefix.to_string();
    let tool_name = display_tool_name(&call.tool_name);
    let available =
        (width as usize).saturating_sub(item_prefix.chars().count() + tool_name.len() + 2);
    let detail_width = width
        .saturating_sub(detail_prefix.chars().count() as u16)
        .max(1) as usize;
    let args_text = if call.args.is_empty() {
        "(no args)".to_string()
    } else {
        call.args.clone()
    };
    let header_available = available.max(10);
    let args_len = args_text.chars().count();
    if args_len <= header_available {
        lines.push(Line::from(vec![
            item_prefix.dim(),
            tool_name.bold(),
            ": ".into(),
            args_text.dim(),
        ]));
    } else {
        lines.push(Line::from(vec![
            item_prefix.dim(),
            tool_name.bold(),
            ":".into(),
        ]));
        for line in wrap_plain_text_lines(&args_text, detail_width) {
            lines.push(Line::from(vec![detail_prefix.clone().dim(), line.dim()]));
        }
    }

    match &call.result {
        ToolCallResult::Running => {
            lines.push(Line::from(vec![
                detail_prefix.dim(),
                "Waiting…".dim().italic(),
            ]));
        }
        ToolCallResult::Success(output) => {
            if output.is_empty() {
                lines.push(Line::from(vec![detail_prefix.dim(), "(no output)".dim()]));
                return;
            }
            let mut output_lines = output.lines();
            let first_line = output_lines.next().unwrap_or("");
            let truncated = truncate_to_n_chars(first_line, detail_width);
            let truncated_more = first_line.chars().count() > detail_width;
            let has_more = output_lines.next().is_some();
            lines.push(Line::from(vec![
                detail_prefix.clone().dim(),
                truncated.into(),
            ]));
            if has_more || truncated_more {
                lines.push(Line::from(vec![detail_prefix.dim(), "...".dim()]));
            }
        }
        ToolCallResult::Error(err) => {
            if err.is_empty() {
                lines.push(Line::from(vec![detail_prefix.dim(), "Error".red()]));
                return;
            }
            let mut err_lines = err.lines();
            let first_line = err_lines.next().unwrap_or("");
            let truncated = truncate_to_n_chars(first_line, detail_width);
            let truncated_more = first_line.chars().count() > detail_width;
            let has_more = err_lines.next().is_some();
            lines.push(Line::from(vec![
                detail_prefix.clone().dim(),
                truncated.red(),
            ]));
            if has_more || truncated_more {
                lines.push(Line::from(vec![detail_prefix.dim(), "...".red()]));
            }
        }
        ToolCallResult::Rejected(reason) => {
            let msg = format!("Tool use rejected: {reason}");
            let truncated = truncate_to_n_chars(&msg, detail_width);
            let truncated_more = msg.chars().count() > detail_width;
            lines.push(Line::from(vec![
                detail_prefix.clone().dim(),
                truncated.dim(),
            ]));
            if truncated_more {
                lines.push(Line::from(vec![detail_prefix.dim(), "...".dim()]));
            }
        }
    }
}

/// Pending tool call waiting for result.
struct PendingToolCall {
    kind: ToolCallKind,
    tool_name: String,
    args: String,
}

/// Extract tool call info from raw events for display.
/// Matches Begin/End events to show tool calls with their results.
fn extract_tool_calls(raw_events: &[EventMsg]) -> Vec<ToolCallInfo> {
    use std::collections::HashMap;
    use std::collections::HashSet;

    let mut pending: HashMap<String, PendingToolCall> = HashMap::new();
    let mut tool_calls = Vec::new();
    let mut seen_call_ids: HashSet<String> = HashSet::new();
    let mut web_search_fallback_counter = 0usize;

    for event in raw_events {
        match event {
            EventMsg::ItemStarted(start) => {
                if let TurnItem::WebSearch(item) = &start.item
                    && !seen_call_ids.contains(&item.id)
                {
                    seen_call_ids.insert(item.id.clone());
                    pending.insert(
                        item.id.clone(),
                        PendingToolCall {
                            kind: ToolCallKind::WebSearch,
                            tool_name: "web_search".to_string(),
                            args: item.query.clone(),
                        },
                    );
                }
            }
            EventMsg::ItemCompleted(done) => {
                if let TurnItem::WebSearch(item) = &done.item {
                    if let Some(mut call) = pending.remove(&item.id) {
                        call.args = item.query.clone();
                        tool_calls.push(ToolCallInfo {
                            kind: call.kind,
                            tool_name: call.tool_name,
                            args: call.args,
                            result: ToolCallResult::Success("Search completed".to_string()),
                        });
                    } else if !seen_call_ids.contains(&item.id) {
                        seen_call_ids.insert(item.id.clone());
                        tool_calls.push(ToolCallInfo {
                            kind: ToolCallKind::WebSearch,
                            tool_name: "web_search".to_string(),
                            args: item.query.clone(),
                            result: ToolCallResult::Success("Search completed".to_string()),
                        });
                    }
                }
            }
            // Begin events - register pending call
            EventMsg::ExecCommandBegin(begin) => {
                if !seen_call_ids.contains(&begin.call_id) {
                    seen_call_ids.insert(begin.call_id.clone());
                    let cmd = strip_bash_lc_and_escape(&begin.command);
                    pending.insert(
                        begin.call_id.clone(),
                        PendingToolCall {
                            kind: ToolCallKind::ExecCommand,
                            tool_name: "shell".to_string(),
                            args: cmd,
                        },
                    );
                }
            }
            EventMsg::McpToolCallBegin(begin) => {
                if !seen_call_ids.contains(&begin.call_id) {
                    seen_call_ids.insert(begin.call_id.clone());
                    pending.insert(
                        begin.call_id.clone(),
                        PendingToolCall {
                            kind: ToolCallKind::Mcp,
                            tool_name: begin.invocation.tool.clone(),
                            args: begin.invocation.server.clone(),
                        },
                    );
                }
            }
            EventMsg::PatchApplyBegin(begin) => {
                if !seen_call_ids.contains(&begin.call_id) {
                    seen_call_ids.insert(begin.call_id.clone());
                    let paths: Vec<String> = begin
                        .changes
                        .keys()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect();
                    let args = if paths.len() == 1 {
                        paths[0].clone()
                    } else {
                        let file_count = paths.len();
                        format!("{file_count} files")
                    };
                    pending.insert(
                        begin.call_id.clone(),
                        PendingToolCall {
                            kind: ToolCallKind::PatchApply,
                            tool_name: "apply_patch".to_string(),
                            args,
                        },
                    );
                }
            }
            EventMsg::WebSearchBegin(begin) => {
                if !seen_call_ids.contains(&begin.call_id) {
                    seen_call_ids.insert(begin.call_id.clone());
                    pending.insert(
                        begin.call_id.clone(),
                        PendingToolCall {
                            kind: ToolCallKind::WebSearch,
                            tool_name: "web_search".to_string(),
                            args: String::new(),
                        },
                    );
                }
            }

            // End events - match with pending and record result
            EventMsg::ExecCommandEnd(end) => {
                if let Some(call) = pending.remove(&end.call_id) {
                    let result = if end.exit_code == 0 {
                        let output = if !end.formatted_output.is_empty() {
                            end.formatted_output.clone()
                        } else if !end.stdout.is_empty() {
                            end.stdout.clone()
                        } else {
                            "(no output)".to_string()
                        };
                        ToolCallResult::Success(output)
                    } else {
                        let error = if !end.stderr.is_empty() {
                            end.stderr.clone()
                        } else {
                            let exit_code = end.exit_code;
                            format!("Exit code: {exit_code}")
                        };
                        ToolCallResult::Error(error)
                    };
                    tool_calls.push(ToolCallInfo {
                        kind: call.kind,
                        tool_name: call.tool_name,
                        args: call.args,
                        result,
                    });
                }
            }
            EventMsg::McpToolCallEnd(end) => {
                if let Some(call) = pending.remove(&end.call_id) {
                    let result = match &end.result {
                        Ok(res) => {
                            if res.is_error.unwrap_or(false) {
                                ToolCallResult::Error("Tool returned error".to_string())
                            } else {
                                ToolCallResult::Success("Completed".to_string())
                            }
                        }
                        Err(e) => ToolCallResult::Error(e.clone()),
                    };
                    tool_calls.push(ToolCallInfo {
                        kind: call.kind,
                        tool_name: call.tool_name,
                        args: call.args,
                        result,
                    });
                }
            }
            EventMsg::PatchApplyEnd(end) => {
                if let Some(call) = pending.remove(&end.call_id) {
                    let result = if end.success {
                        let output = if !end.stdout.is_empty() {
                            end.stdout.clone()
                        } else {
                            let file_count = end.changes.len();
                            let suffix = if file_count == 1 { "" } else { "s" };
                            format!("Wrote {file_count} file{suffix}")
                        };
                        ToolCallResult::Success(output)
                    } else {
                        let error = if !end.stderr.is_empty() {
                            end.stderr.clone()
                        } else {
                            "Patch failed".to_string()
                        };
                        ToolCallResult::Error(error)
                    };
                    tool_calls.push(ToolCallInfo {
                        kind: call.kind,
                        tool_name: call.tool_name,
                        args: call.args,
                        result,
                    });
                }
            }
            EventMsg::WebSearchEnd(end) => {
                if let Some(mut call) = pending.remove(&end.call_id) {
                    call.args = end.query.clone();
                    tool_calls.push(ToolCallInfo {
                        kind: call.kind,
                        tool_name: call.tool_name,
                        args: call.args,
                        result: ToolCallResult::Success("Search completed".to_string()),
                    });
                }
            }
            _ => {}
        }
    }

    for event in raw_events {
        if let EventMsg::RawResponseItem(raw) = event {
            match &raw.item {
                ResponseItem::FunctionCall {
                    name,
                    call_id,
                    arguments,
                    ..
                } => {
                    // Only track if we haven't seen this call_id before (avoid duplicates)
                    if !seen_call_ids.contains(call_id) {
                        seen_call_ids.insert(call_id.clone());
                        pending.insert(
                            call_id.clone(),
                            PendingToolCall {
                                kind: ToolCallKind::ApiFunction,
                                tool_name: name.clone(),
                                args: arguments.clone(),
                            },
                        );
                    }
                }
                ResponseItem::CustomToolCall {
                    name,
                    call_id,
                    input,
                    ..
                } => {
                    // Only track if we haven't seen this call_id before (avoid duplicates)
                    if !seen_call_ids.contains(call_id) {
                        seen_call_ids.insert(call_id.clone());
                        pending.insert(
                            call_id.clone(),
                            PendingToolCall {
                                kind: ToolCallKind::ApiCustom,
                                tool_name: name.clone(),
                                args: input.clone(),
                            },
                        );
                    }
                }
                ResponseItem::WebSearchCall { id, action, .. } => {
                    let call_id = id.clone().unwrap_or_else(|| {
                        web_search_fallback_counter += 1;
                        format!("web_search_{web_search_fallback_counter}")
                    });
                    if !seen_call_ids.contains(&call_id) {
                        seen_call_ids.insert(call_id.clone());
                        tool_calls.push(ToolCallInfo {
                            kind: ToolCallKind::WebSearch,
                            tool_name: "web_search".to_string(),
                            args: web_search_args(action),
                            result: ToolCallResult::Success("Search completed".to_string()),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    for event in raw_events {
        if let EventMsg::RawResponseItem(raw) = event {
            match &raw.item {
                ResponseItem::FunctionCallOutput { call_id, output } => {
                    // Match with pending function call
                    if let Some(call) = pending.remove(call_id) {
                        let result = if output.success.unwrap_or(true) {
                            ToolCallResult::Success(output.content.clone())
                        } else {
                            ToolCallResult::Error(output.content.clone())
                        };
                        tool_calls.push(ToolCallInfo {
                            kind: call.kind,
                            tool_name: call.tool_name,
                            args: call.args,
                            result,
                        });
                    }
                }
                ResponseItem::CustomToolCallOutput { call_id, output } => {
                    // Match with pending custom tool call
                    if let Some(call) = pending.remove(call_id) {
                        tool_calls.push(ToolCallInfo {
                            kind: call.kind,
                            tool_name: call.tool_name,
                            args: call.args,
                            result: ToolCallResult::Success(output.clone()),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    // Add any remaining pending calls as "Running"
    for (_, call) in pending {
        tool_calls.push(ToolCallInfo {
            kind: call.kind,
            tool_name: call.tool_name,
            args: call.args,
            result: ToolCallResult::Running,
        });
    }

    tool_calls
}

/// Format duration for display.
fn format_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs_f64();
    if secs < 60.0 {
        format!("{secs:.1}s")
    } else {
        let mins = secs / 60.0;
        format!("{mins:.1}m")
    }
}

impl HistoryCell for SubAgentCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.render_with_verbosity(width, false)
    }

    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.render_with_verbosity(width, true)
    }

    fn transcript_animation_tick(&self) -> Option<u64> {
        if self.is_active() {
            self.active_start_time()
                .map(|t| t.elapsed().as_millis() as u64 / 300)
        } else {
            None
        }
    }
}

impl SubAgentCell {
    /// Render the cell with explicit verbosity flag.
    fn render_with_verbosity(&self, width: u16, verbose: bool) -> Vec<Line<'static>> {
        if self.agents.is_empty() {
            return Vec::new();
        }

        // Single agent: render directly
        if self.agents.len() == 1 {
            return self.render_single_agent(&self.agents[0], width, verbose);
        }

        // Multiple agents: render as a group
        self.render_group(width, verbose)
    }

    /// Render a single agent entry.
    fn render_single_agent(
        &self,
        agent: &SubAgentEntry,
        width: u16,
        verbose: bool,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        // Status bullet
        let status_icon = match &agent.status {
            SubAgentStatus::Running => spinner(agent.start_time, self.animations_enabled),
            SubAgentStatus::Completed(_) => "•".green().bold(),
            SubAgentStatus::Error(_) => "•".red().bold(),
        };

        // Truncate description to fit
        let desc_width = (width as usize).saturating_sub(30).max(20);
        let truncated_desc = truncate_to_n_chars(&agent.description, desc_width);

        // Header: "• AgentType(Description)"
        let header: Line<'static> = vec![
            status_icon,
            " ".into(),
            agent.agent_type.clone().bold(),
            "(".into(),
            truncated_desc.dim(),
            ")".into(),
        ]
        .into();
        lines.push(header);

        let tool_calls = extract_tool_calls(&agent.raw_events);
        let tool_count = tool_calls.len();
        let visible_tool_calls: Vec<&ToolCallInfo> = tool_calls
            .iter()
            .filter(|call| should_display_tool_call(call))
            .collect();
        let visible_count = visible_tool_calls.len();
        let tool_word = if tool_count == 1 {
            "tool use"
        } else {
            "tool uses"
        };
        let duration_str = agent
            .duration
            .map(format_duration)
            .unwrap_or_else(|| "?".to_string());

        if verbose {
            // EXPANDED MODE: Show Prompt, tool calls with details, Response, then Done

            // Prompt section
            if let Some(prompt) = &agent.prompt {
                lines.push(Line::from(vec!["  ├ ".dim(), "Prompt:".green().bold()]));
                push_wrapped_lines(&mut lines, "  │   ", prompt, width);
            }

            // Tool calls section with results
            if tool_count > 0 {
                let header_text = tool_calls_header_text(tool_count, visible_count);
                lines.push(Line::from(vec!["  ├ ".dim(), header_text.cyan().bold()]));
                if visible_tool_calls.is_empty() {
                    lines.push(Line::from(vec![
                        "  │   ".dim(),
                        "No Shell/Apply Patch/Read File calls to show.".dim(),
                    ]));
                } else {
                    for call in &visible_tool_calls {
                        render_tool_call_block(&mut lines, "  │   ", "  │     ", call, width);
                    }
                }
            }

            // Response section (from status)
            match &agent.status {
                SubAgentStatus::Completed(Some(response)) if !response.is_empty() => {
                    lines.push(Line::from(vec!["  ├ ".dim(), "Response:".green().bold()]));
                    push_wrapped_lines(&mut lines, "  │   ", response, width);
                }
                SubAgentStatus::Error(msg) => {
                    lines.push(Line::from(vec!["  ├ ".dim(), "Error:".red().bold()]));
                    push_wrapped_lines_red(&mut lines, "  │   ", msg, width);
                }
                _ => {}
            }

            // Done line
            let done_text = match &agent.status {
                SubAgentStatus::Running => "Running...".to_string(),
                SubAgentStatus::Completed(_) => {
                    format!("Done ({tool_count} {tool_word} · {duration_str})")
                }
                SubAgentStatus::Error(_) => {
                    format!("Failed ({tool_count} {tool_word} · {duration_str})")
                }
            };
            lines.push(Line::from(vec!["  └ ".dim(), done_text.dim()]));
        } else {
            // COLLAPSED MODE: Show current tool (running) or Done stats
            let detail_line: Line<'static> = match &agent.status {
                SubAgentStatus::Running => {
                    let text = if let Some(last_call) = visible_tool_calls.last() {
                        let tool_name = display_tool_name(&last_call.tool_name);
                        let args = truncate_to_n_chars(&last_call.args, 60);
                        if last_call.args.is_empty() {
                            tool_name
                        } else {
                            format!("{tool_name}: {args}")
                        }
                    } else {
                        "Running...".to_string()
                    };
                    Line::from(vec!["  └ ".dim(), text.dim()])
                }
                SubAgentStatus::Completed(_) => Line::from(
                    format!("  └ Done ({tool_count} {tool_word} · {duration_str})").dim(),
                ),
                SubAgentStatus::Error(msg) => {
                    let short_msg = truncate_to_n_chars(msg, 40);
                    Line::from(format!("  └ Error: {short_msg} ({duration_str})").red())
                }
            };
            lines.push(detail_line);
        }

        lines
    }

    /// Render multiple agents as a group.
    fn render_group(&self, width: u16, verbose: bool) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        // Count by status
        let running_count = self
            .agents
            .iter()
            .filter(|a| matches!(a.status, SubAgentStatus::Running))
            .count();
        let error_count = self
            .agents
            .iter()
            .filter(|a| matches!(a.status, SubAgentStatus::Error(_)))
            .count();
        let total = self.agents.len();

        // Group header bullet
        let bullet = if running_count > 0 {
            spinner(self.active_start_time(), self.animations_enabled)
        } else if error_count > 0 {
            "•".red().bold()
        } else {
            "•".green().bold()
        };

        // Status text
        let status_text = if running_count > 0 {
            if running_count == total {
                format!("Running {total} agents...")
            } else {
                format!("Running {running_count} of {total} agents...")
            }
        } else if error_count > 0 {
            let completed = total - error_count;
            format!("{total} agents: {completed} completed, {error_count} failed")
        } else {
            format!("{total} agents completed")
        };

        lines.push(Line::from(vec![bullet, " ".into(), status_text.into()]));

        if verbose {
            // EXPANDED: Show each agent with tree prefixes
            let agent_count = self.agents.len();
            for (i, agent) in self.agents.iter().enumerate() {
                let is_last = i == agent_count - 1;
                self.render_agent_in_group(&mut lines, agent, is_last, width);
            }
        } else {
            // COLLAPSED: Show each agent with summary info
            let agent_count = self.agents.len();
            for (i, agent) in self.agents.iter().enumerate() {
                let is_last = i == agent_count - 1;
                let prefix = if is_last { "└─ " } else { "├─ " };

                // Agent description and stats
                let tool_count = extract_tool_calls(&agent.raw_events).len();
                let tool_word = if tool_count == 1 {
                    "tool use"
                } else {
                    "tool uses"
                };
                let duration_str = agent
                    .duration
                    .map(format_duration)
                    .unwrap_or_else(|| "?".to_string());

                let desc_width = (width as usize).saturating_sub(40).max(10);
                let truncated_desc = truncate_to_n_chars(&agent.description, desc_width);

                lines.push(Line::from(vec![
                    prefix.dim(),
                    truncated_desc.into(),
                    format!(" · {tool_count} {tool_word} · {duration_str}").dim(),
                ]));

                // Status line
                let continuation = if is_last { "   " } else { "│  " };
                let status_text = match &agent.status {
                    SubAgentStatus::Running => "Running...".to_string(),
                    SubAgentStatus::Completed(_) => "Done".to_string(),
                    SubAgentStatus::Error(msg) => {
                        let short_msg = truncate_to_n_chars(msg, 30);
                        format!("Error: {short_msg}")
                    }
                };
                lines.push(Line::from(vec![
                    continuation.dim(),
                    "└ ".dim(),
                    status_text.dim(),
                ]));
            }
        }

        lines
    }

    /// Render a single agent within the group with tree prefixes.
    fn render_agent_in_group(
        &self,
        lines: &mut Vec<Line<'static>>,
        agent: &SubAgentEntry,
        is_last: bool,
        width: u16,
    ) {
        let prefix = if is_last { "└─ " } else { "├─ " };
        let continuation = if is_last { "   " } else { "│  " };

        // Status icon
        let status_icon = match &agent.status {
            SubAgentStatus::Running => spinner(agent.start_time, self.animations_enabled),
            SubAgentStatus::Completed(_) => "•".green().bold(),
            SubAgentStatus::Error(_) => "•".red().bold(),
        };

        // Agent header
        let desc_width = (width as usize).saturating_sub(20).max(10);
        let truncated_desc = truncate_to_n_chars(&agent.description, desc_width);
        lines.push(Line::from(vec![
            prefix.dim(),
            status_icon,
            " ".into(),
            agent.agent_type.clone().bold(),
            "(".into(),
            truncated_desc.dim(),
            ")".into(),
        ]));

        let tool_calls = extract_tool_calls(&agent.raw_events);
        let tool_count = tool_calls.len();
        let visible_tool_calls: Vec<&ToolCallInfo> = tool_calls
            .iter()
            .filter(|call| should_display_tool_call(call))
            .collect();
        let visible_count = visible_tool_calls.len();
        let section_prefix = format!("{continuation}├ ");
        let content_prefix = format!("{continuation}│   ");
        let detail_prefix = format!("{continuation}│     ");

        // Prompt section
        if let Some(prompt) = &agent.prompt {
            lines.push(Line::from(vec![
                section_prefix.clone().dim(),
                "Prompt:".green().bold(),
            ]));
            push_wrapped_lines(lines, &content_prefix, prompt, width);
        }

        // Tool calls with results
        if tool_count > 0 {
            let header_text = tool_calls_header_text(tool_count, visible_count);
            lines.push(Line::from(vec![
                section_prefix.clone().dim(),
                header_text.cyan().bold(),
            ]));
            if visible_tool_calls.is_empty() {
                lines.push(Line::from(vec![
                    content_prefix.clone().dim(),
                    "No Shell/Apply Patch/Read File calls to show.".dim(),
                ]));
            } else {
                for call in &visible_tool_calls {
                    render_tool_call_block(lines, &content_prefix, &detail_prefix, call, width);
                }
            }
        }

        // Response section (from status)
        match &agent.status {
            SubAgentStatus::Completed(Some(response)) if !response.is_empty() => {
                lines.push(Line::from(vec![
                    section_prefix.dim(),
                    "Response:".green().bold(),
                ]));
                push_wrapped_lines(lines, &content_prefix, response, width);
            }
            SubAgentStatus::Error(msg) => {
                lines.push(Line::from(vec![
                    section_prefix.dim(),
                    "Error:".red().bold(),
                ]));
                push_wrapped_lines_red(lines, &content_prefix, msg, width);
            }
            _ => {}
        }

        // Status line
        let tool_word = if tool_count == 1 {
            "tool use"
        } else {
            "tool uses"
        };
        let status_line = match &agent.status {
            SubAgentStatus::Running => "Running...".to_string(),
            SubAgentStatus::Completed(_) => {
                let duration_str = agent
                    .duration
                    .map(format_duration)
                    .unwrap_or_else(|| "?".to_string());
                format!("Done ({tool_count} {tool_word} · {duration_str})")
            }
            SubAgentStatus::Error(_) => {
                let duration_str = agent
                    .duration
                    .map(format_duration)
                    .unwrap_or_else(|| "?".to_string());
                format!("Failed ({tool_count} {tool_word} · {duration_str})")
            }
        };
        lines.push(Line::from(vec![
            continuation.dim(),
            "└ ".dim(),
            status_line.dim(),
        ]));
    }
}
