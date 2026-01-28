use super::model::SubAgentCell;
use super::model::SubAgentEntry;
use super::model::SubAgentStatus;
use crate::exec_cell::spinner;
use crate::exec_command::strip_bash_lc_and_escape;
use crate::history_cell::HistoryCell;
use crate::verbosity::DisplayVerbosity;
use crate::verbosity::RenderContext;
use codex_core::protocol::EventMsg;
use codex_core::protocol::SubAgentBeginEvent;
use ratatui::prelude::*;

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

/// Extracted tool call info for rendering.
#[derive(Debug, Clone)]
struct ToolCallInfo {
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

/// Pending tool call waiting for result.
struct PendingToolCall {
    tool_name: String,
    args: String,
}

/// Extract tool call info from raw events for display.
/// Matches Begin/End events to show tool calls with their results.
fn extract_tool_calls(raw_events: &[EventMsg]) -> Vec<ToolCallInfo> {
    use std::collections::HashMap;

    let mut pending: HashMap<String, PendingToolCall> = HashMap::new();
    let mut tool_calls = Vec::new();

    for event in raw_events {
        match event {
            // Begin events - register pending call
            EventMsg::ExecCommandBegin(begin) => {
                let cmd = strip_bash_lc_and_escape(&begin.command);
                pending.insert(
                    begin.call_id.clone(),
                    PendingToolCall {
                        tool_name: "Shell".to_string(),
                        args: cmd,
                    },
                );
            }
            EventMsg::McpToolCallBegin(begin) => {
                pending.insert(
                    begin.call_id.clone(),
                    PendingToolCall {
                        tool_name: begin.invocation.tool.clone(),
                        args: begin.invocation.server.clone(),
                    },
                );
            }
            EventMsg::PatchApplyBegin(begin) => {
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
                        tool_name: "Write".to_string(),
                        args,
                    },
                );
            }
            EventMsg::WebSearchBegin(begin) => {
                pending.insert(
                    begin.call_id.clone(),
                    PendingToolCall {
                        tool_name: "WebSearch".to_string(),
                        args: String::new(),
                    },
                );
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
                        tool_name: call.tool_name,
                        args: call.args,
                        result: ToolCallResult::Success("Search completed".to_string()),
                    });
                }
            }
            _ => {}
        }
    }

    // Add any remaining pending calls as "Running"
    for (_, call) in pending {
        tool_calls.push(ToolCallInfo {
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
    fn display_lines(&self, ctx: RenderContext) -> Vec<Line<'static>> {
        self.render_with_verbosity(ctx.width, ctx.verbosity == DisplayVerbosity::Verbose)
    }

    fn transcript_lines(&self, ctx: RenderContext) -> Vec<Line<'static>> {
        self.display_lines(ctx)
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

        // Hint text for expand/collapse
        let hint = if verbose {
            "(ctrl+o to collapse)"
        } else {
            "(ctrl+o to expand)"
        };

        // Truncate description to fit
        let desc_width = (width as usize).saturating_sub(30).max(20);
        let truncated_desc = truncate_to_n_chars(&agent.description, desc_width);

        // Header: "• AgentType(Description) (ctrl+o to expand)"
        let header: Line<'static> = vec![
            status_icon,
            " ".into(),
            agent.agent_type.clone().bold(),
            "(".into(),
            truncated_desc.dim(),
            ") ".into(),
            hint.dim(),
        ]
        .into();
        lines.push(header);

        let tool_calls = extract_tool_calls(&agent.raw_events);
        let tool_count = tool_calls.len();
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
                for line in prompt.lines().take(3) {
                    let truncated = truncate_to_n_chars(line, (width as usize).saturating_sub(6));
                    lines.push(Line::from(vec!["  │   ".dim(), truncated.into()]));
                }
                if prompt.lines().count() > 3 {
                    lines.push(Line::from(vec!["  │   ".dim(), "...".dim()]));
                }
            }

            // Tool calls section with results
            for call in &tool_calls {
                let available = (width as usize).saturating_sub(8 + call.tool_name.len() + 2);
                // Tool call header: "  ├ Shell(echo "Hello World")"
                lines.push(Line::from(vec![
                    "  ├ ".dim(),
                    call.tool_name.clone().bold(),
                    format!("({})", truncate_to_n_chars(&call.args, available)).into(),
                ]));

                // Tool result - first line gets "├", rest are indented
                match &call.result {
                    ToolCallResult::Running => {
                        lines.push(Line::from(vec!["    ".into(), "Waiting…".dim().italic()]));
                    }
                    ToolCallResult::Success(output) => {
                        let mut output_lines = output.lines().take(5);
                        // First line with marker
                        if let Some(first) = output_lines.next() {
                            let truncated =
                                truncate_to_n_chars(first, (width as usize).saturating_sub(6));
                            lines.push(Line::from(vec!["  ├ ".dim(), truncated.into()]));
                        }
                        // Rest indented without marker
                        for line in output_lines {
                            let truncated =
                                truncate_to_n_chars(line, (width as usize).saturating_sub(6));
                            lines.push(Line::from(vec!["    ".into(), truncated.into()]));
                        }
                        if output.lines().count() > 5 {
                            lines.push(Line::from(vec!["    ".into(), "...".dim()]));
                        }
                    }
                    ToolCallResult::Error(err) => {
                        let truncated =
                            truncate_to_n_chars(err, (width as usize).saturating_sub(6));
                        lines.push(Line::from(vec!["  ├ ".dim(), truncated.red()]));
                    }
                    ToolCallResult::Rejected(reason) => {
                        let msg = format!("Tool use rejected: {reason}");
                        let truncated =
                            truncate_to_n_chars(&msg, (width as usize).saturating_sub(6));
                        lines.push(Line::from(vec!["  ├ ".dim(), truncated.dim()]));
                    }
                }
            }

            // Response section (from status)
            match &agent.status {
                SubAgentStatus::Completed(Some(response)) if !response.is_empty() => {
                    lines.push(Line::from(vec!["  ├ ".dim(), "Response:".green().bold()]));
                    for line in response.lines().take(5) {
                        let truncated =
                            truncate_to_n_chars(line, (width as usize).saturating_sub(6));
                        lines.push(Line::from(vec!["  │   ".dim(), truncated.into()]));
                    }
                    if response.lines().count() > 5 {
                        lines.push(Line::from(vec!["  │   ".dim(), "...".dim()]));
                    }
                }
                SubAgentStatus::Error(msg) => {
                    lines.push(Line::from(vec!["  ├ ".dim(), "Error:".red().bold()]));
                    let truncated = truncate_to_n_chars(msg, (width as usize).saturating_sub(6));
                    lines.push(Line::from(vec!["  │   ".dim(), truncated.red()]));
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
                    let text = if let Some(last_call) = tool_calls.last() {
                        let tool_name = &last_call.tool_name;
                        let args = truncate_to_n_chars(&last_call.args, 60);
                        format!("{tool_name}({args})")
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

        // Hint text
        let hint = if verbose {
            "(ctrl+o to collapse)"
        } else {
            "(ctrl+o to expand)"
        };

        // Status text
        let status_text = if running_count > 0 {
            if running_count == total {
                format!("Running {total} agents... {hint}")
            } else {
                format!("Running {running_count} of {total} agents... {hint}")
            }
        } else if error_count > 0 {
            let completed = total - error_count;
            format!("{total} agents: {completed} completed, {error_count} failed {hint}")
        } else {
            format!("{total} agents completed {hint}")
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

        // Prompt section
        if let Some(prompt) = &agent.prompt {
            lines.push(Line::from(vec![
                continuation.dim(),
                "├ ".dim(),
                "Prompt:".green().bold(),
            ]));
            for line in prompt.lines().take(3) {
                let truncated = truncate_to_n_chars(line, (width as usize).saturating_sub(8));
                lines.push(Line::from(vec![
                    continuation.dim(),
                    "│   ".dim(),
                    truncated.into(),
                ]));
            }
            if prompt.lines().count() > 3 {
                lines.push(Line::from(vec![
                    continuation.dim(),
                    "│   ".dim(),
                    "...".dim(),
                ]));
            }
        }

        // Tool calls with results
        let tool_calls = extract_tool_calls(&agent.raw_events);
        for call in &tool_calls {
            let available = (width as usize).saturating_sub(10 + call.tool_name.len());
            // Tool call header
            lines.push(Line::from(vec![
                continuation.dim(),
                "├ ".dim(),
                call.tool_name.clone().bold(),
                format!("({})", truncate_to_n_chars(&call.args, available)).into(),
            ]));

            // Tool result - first line gets "├", rest are indented
            match &call.result {
                ToolCallResult::Running => {
                    lines.push(Line::from(vec![
                        continuation.dim(),
                        "  ".into(),
                        "Waiting…".dim().italic(),
                    ]));
                }
                ToolCallResult::Success(output) => {
                    let mut output_lines = output.lines().take(3);
                    // First line with marker
                    if let Some(first) = output_lines.next() {
                        let truncated =
                            truncate_to_n_chars(first, (width as usize).saturating_sub(8));
                        lines.push(Line::from(vec![
                            continuation.dim(),
                            "├ ".dim(),
                            truncated.into(),
                        ]));
                    }
                    // Rest indented without marker
                    for line in output_lines {
                        let truncated =
                            truncate_to_n_chars(line, (width as usize).saturating_sub(8));
                        lines.push(Line::from(vec![
                            continuation.dim(),
                            "  ".into(),
                            truncated.into(),
                        ]));
                    }
                    if output.lines().count() > 3 {
                        lines.push(Line::from(vec![
                            continuation.dim(),
                            "  ".into(),
                            "...".dim(),
                        ]));
                    }
                }
                ToolCallResult::Error(err) => {
                    let truncated = truncate_to_n_chars(err, (width as usize).saturating_sub(8));
                    lines.push(Line::from(vec![
                        continuation.dim(),
                        "├ ".dim(),
                        truncated.red(),
                    ]));
                }
                ToolCallResult::Rejected(reason) => {
                    let msg = format!("Tool use rejected: {reason}");
                    let truncated = truncate_to_n_chars(&msg, (width as usize).saturating_sub(8));
                    lines.push(Line::from(vec![
                        continuation.dim(),
                        "├ ".dim(),
                        truncated.dim(),
                    ]));
                }
            }
        }

        // Response section (from status)
        match &agent.status {
            SubAgentStatus::Completed(Some(response)) if !response.is_empty() => {
                lines.push(Line::from(vec![
                    continuation.dim(),
                    "├ ".dim(),
                    "Response:".green().bold(),
                ]));
                for line in response.lines().take(3) {
                    let truncated = truncate_to_n_chars(line, (width as usize).saturating_sub(8));
                    lines.push(Line::from(vec![
                        continuation.dim(),
                        "│   ".dim(),
                        truncated.into(),
                    ]));
                }
                if response.lines().count() > 3 {
                    lines.push(Line::from(vec![
                        continuation.dim(),
                        "│   ".dim(),
                        "...".dim(),
                    ]));
                }
            }
            SubAgentStatus::Error(msg) => {
                lines.push(Line::from(vec![
                    continuation.dim(),
                    "├ ".dim(),
                    "Error:".red().bold(),
                ]));
                let truncated = truncate_to_n_chars(msg, (width as usize).saturating_sub(8));
                lines.push(Line::from(vec![
                    continuation.dim(),
                    "│   ".dim(),
                    truncated.red(),
                ]));
            }
            _ => {}
        }

        // Status line
        let tool_count = tool_calls.len();
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
