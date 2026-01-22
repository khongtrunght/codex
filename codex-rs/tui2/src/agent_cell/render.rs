use super::model::SubAgentCell;
use super::model::SubAgentEntry;
use super::model::SubAgentStatus;
use crate::exec_cell::spinner;
use crate::exec_command::strip_bash_lc_and_escape;
use crate::history_cell::HistoryCell;
use crate::history_cell::RenderContext;
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
                    format!("{} files", paths.len())
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
                            format!("Exit code: {}", end.exit_code)
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
                            format!(
                                "Wrote {} file{}",
                                file_count,
                                if file_count == 1 { "" } else { "s" }
                            )
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
        self.render_with_verbosity(ctx.width, ctx.is_verbose())
    }

    fn transcript_lines_with_joiners(
        &self,
        ctx: RenderContext,
    ) -> crate::history_cell::TranscriptLinesWithJoiners {
        let lines = self.render_with_verbosity(ctx.width, ctx.is_verbose());
        crate::history_cell::TranscriptLinesWithJoiners {
            joiner_before: vec![None; lines.len()],
            lines,
        }
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
                        format!(
                            "{}({})",
                            last_call.tool_name,
                            truncate_to_n_chars(&last_call.args, 60)
                        )
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
            format!(
                "{} agents: {} completed, {} failed {hint}",
                total,
                total - error_count,
                error_count
            )
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
                        format!("Error: {}", truncate_to_n_chars(msg, 30))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verbosity::RenderContext;
    use codex_core::protocol::AgentStatus;
    use codex_core::protocol::ExecCommandBeginEvent;
    use codex_core::protocol::ExecCommandEndEvent;
    use codex_core::protocol::ExecCommandSource;
    use codex_core::protocol::SubAgentBeginEvent;
    use codex_protocol::ThreadId;
    use std::path::PathBuf;

    /// Helper to render lines to plain strings for assertions.
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

    #[test]
    fn subagent_cell_running_displays_spinner_and_status() {
        let begin_event = SubAgentBeginEvent {
            call_id: "call-123".to_string(),
            agent_type: "Explore".to_string(),
            description: "Find test files".to_string(),
            prompt: String::new(),
            sender_thread_id: ThreadId::default(),
            resumed: false,
        };
        let cell = new_subagent_cell(begin_event, false);
        let lines = cell.display_lines(RenderContext::new(80));
        let rendered = render_lines(&lines);

        // With animations disabled, spinner shows as "•"
        assert_eq!(rendered.len(), 2);
        assert!(rendered[0].contains("Explore"));
        assert!(rendered[0].contains("Find test files"));
        assert!(rendered[0].contains("ctrl+o to expand"));
        assert!(rendered[1].contains("Running"));
    }

    #[test]
    fn subagent_cell_completed_displays_tool_count_and_duration() {
        let begin_event = SubAgentBeginEvent {
            call_id: "call-456".to_string(),
            agent_type: "Plan".to_string(),
            description: "Design implementation".to_string(),
            prompt: String::new(),
            sender_thread_id: ThreadId::default(),
            resumed: false,
        };
        let mut cell = new_subagent_cell(begin_event, false);

        // Add some tool events to the first agent
        if let Some(agent) = cell.find_agent_mut("call-456") {
            agent.add_event(EventMsg::ExecCommandBegin(ExecCommandBeginEvent {
                call_id: "exec-1".to_string(),
                process_id: None,
                turn_id: "turn-1".to_string(),
                command: vec!["ls".to_string(), "-la".to_string()],
                cwd: PathBuf::from("/tmp"),
                parsed_cmd: vec![],
                source: ExecCommandSource::Agent,
                interaction_input: None,
            }));
            agent.add_event(EventMsg::ExecCommandBegin(ExecCommandBeginEvent {
                call_id: "exec-2".to_string(),
                process_id: None,
                turn_id: "turn-1".to_string(),
                command: vec!["cat".to_string(), "file.txt".to_string()],
                cwd: PathBuf::from("/tmp"),
                parsed_cmd: vec![],
                source: ExecCommandSource::Agent,
                interaction_input: None,
            }));
            agent.complete_with_status(&AgentStatus::Completed(None));
        }

        let lines = cell.display_lines(RenderContext::new(80));
        let rendered = render_lines(&lines);

        assert_eq!(rendered.len(), 2);
        assert!(rendered[0].contains("Plan"));
        assert!(rendered[0].contains("ctrl+o to expand"));
        // Should show "2 tool uses" and duration
        assert!(rendered[1].contains("Done"));
        assert!(rendered[1].contains("2 tool uses"));
    }

    #[test]
    fn subagent_cell_error_displays_error_message() {
        let begin_event = SubAgentBeginEvent {
            call_id: "call-789".to_string(),
            agent_type: "general".to_string(),
            description: "Process request".to_string(),
            prompt: String::new(),
            sender_thread_id: ThreadId::default(),
            resumed: false,
        };
        let mut cell = new_subagent_cell(begin_event, false);

        // Complete with error
        if let Some(agent) = cell.find_agent_mut("call-789") {
            agent.complete_with_status(&AgentStatus::Errored("Something went wrong".to_string()));
        }

        let lines = cell.display_lines(RenderContext::new(80));
        let rendered = render_lines(&lines);

        assert_eq!(rendered.len(), 2);
        assert!(rendered[0].contains("general"));
        assert!(rendered[1].contains("Error"));
        assert!(rendered[1].contains("Something went wrong"));
    }

    #[test]
    fn subagent_cell_verbose_shows_tool_details() {
        let begin_event = SubAgentBeginEvent {
            call_id: "call-verbose".to_string(),
            agent_type: "Explore".to_string(),
            description: "Search codebase".to_string(),
            prompt: String::new(),
            sender_thread_id: ThreadId::default(),
            resumed: false,
        };
        let mut cell = new_subagent_cell(begin_event, false);

        // Add tool events (both Begin and End to show result)
        if let Some(agent) = cell.find_agent_mut("call-verbose") {
            agent.add_event(EventMsg::ExecCommandBegin(ExecCommandBeginEvent {
                call_id: "exec-1".to_string(),
                process_id: None,
                turn_id: "turn-1".to_string(),
                command: vec!["grep".to_string(), "-r".to_string(), "pattern".to_string()],
                cwd: PathBuf::from("/tmp"),
                parsed_cmd: vec![],
                source: ExecCommandSource::Agent,
                interaction_input: None,
            }));
            // Add the End event to show the result
            agent.add_event(EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                call_id: "exec-1".to_string(),
                process_id: None,
                turn_id: "turn-1".to_string(),
                command: vec!["grep".to_string(), "-r".to_string(), "pattern".to_string()],
                cwd: PathBuf::from("/tmp"),
                parsed_cmd: vec![],
                source: ExecCommandSource::Agent,
                interaction_input: None,
                stdout: "file1.rs:10: match found\nfile2.rs:20: another match".to_string(),
                stderr: String::new(),
                aggregated_output: String::new(),
                exit_code: 0,
                duration: std::time::Duration::from_millis(100),
                formatted_output: "file1.rs:10: match found\nfile2.rs:20: another match"
                    .to_string(),
            }));
            agent
                .complete_with_status(&AgentStatus::Completed(Some("Found 5 matches".to_string())));
        }

        // Compact mode should have 2 lines
        let compact_lines = cell.display_lines(RenderContext::new(80));
        assert_eq!(compact_lines.len(), 2);
        let compact_rendered = render_lines(&compact_lines);
        assert!(compact_rendered[0].contains("ctrl+o to expand"));

        // Verbose mode should show tool details and response
        let ctx = RenderContext::with_verbosity(80, crate::verbosity::DisplayVerbosity::Verbose);
        let verbose_lines = cell.display_lines(ctx);
        assert!(
            verbose_lines.len() > 2,
            "Verbose mode should show more lines: got {}",
            verbose_lines.len()
        );

        let rendered = render_lines(&verbose_lines);
        assert!(rendered[0].contains("ctrl+o to collapse"));
        assert!(
            rendered.iter().any(|l| l.contains("Shell")),
            "Verbose mode should show tool name 'Shell': {rendered:?}"
        );
        assert!(
            rendered.iter().any(|l| l.contains("match found")),
            "Verbose mode should show tool output: {rendered:?}"
        );
        assert!(
            rendered.iter().any(|l| l.contains("Response:")),
            "Verbose mode should show Response section: {rendered:?}"
        );
        assert!(
            rendered.iter().any(|l| l.contains("Found 5 matches")),
            "Verbose mode should show response content: {rendered:?}"
        );
    }

    #[test]
    fn subagent_group_displays_summary() {
        // Create multiple entries
        let entry1 = SubAgentEntry::new(
            "call-1".to_string(),
            "Explore".to_string(),
            "Task 1".to_string(),
            None,
        );
        let mut entry2 = SubAgentEntry::new(
            "call-2".to_string(),
            "Plan".to_string(),
            "Task 2".to_string(),
            None,
        );
        entry2.complete_with_status(&AgentStatus::Completed(None));

        let cell = SubAgentCell::from_entries(vec![entry1, entry2], false);
        let lines = cell.display_lines(RenderContext::new(80));
        let rendered = render_lines(&lines);

        // Should show group summary with hint
        assert!(
            rendered[0].contains("agents"),
            "Group header should mention agents: {rendered:?}"
        );
        assert!(
            rendered[0].contains("ctrl+o"),
            "Group header should show hint: {rendered:?}"
        );
    }

    #[test]
    fn subagent_group_verbose_shows_all_agents() {
        let mut entry1 = SubAgentEntry::new(
            "call-1".to_string(),
            "Explore".to_string(),
            "Task 1".to_string(),
            Some("Search for test files".to_string()),
        );
        entry1.complete_with_status(&AgentStatus::Completed(Some("Result 1".to_string())));

        let mut entry2 = SubAgentEntry::new(
            "call-2".to_string(),
            "Plan".to_string(),
            "Task 2".to_string(),
            Some("Design the architecture".to_string()),
        );
        entry2.complete_with_status(&AgentStatus::Completed(Some("Result 2".to_string())));

        let cell = SubAgentCell::from_entries(vec![entry1, entry2], false);

        // Compact mode
        let compact_lines = cell.display_lines(RenderContext::new(80));
        let compact_rendered = render_lines(&compact_lines);
        assert!(compact_rendered[0].contains("ctrl+o to expand"));

        // Verbose mode should show each agent with Response
        let ctx = RenderContext::with_verbosity(80, crate::verbosity::DisplayVerbosity::Verbose);
        let verbose_lines = cell.display_lines(ctx);
        let verbose_rendered = render_lines(&verbose_lines);

        assert!(verbose_rendered[0].contains("ctrl+o to collapse"));
        assert!(
            verbose_rendered.iter().any(|l| l.contains("Explore")),
            "Should show first agent type"
        );
        assert!(
            verbose_rendered.iter().any(|l| l.contains("Plan")),
            "Should show second agent type"
        );
        assert!(
            verbose_rendered.iter().any(|l| l.contains("Prompt:")),
            "Should show Prompt section: {verbose_rendered:?}"
        );
        assert!(
            verbose_rendered
                .iter()
                .any(|l| l.contains("Search for test files")),
            "Should show prompt content: {verbose_rendered:?}"
        );
        assert!(
            verbose_rendered.iter().any(|l| l.contains("Response:")),
            "Should show Response section: {verbose_rendered:?}"
        );
    }

    // =========================================================================
    // Snapshot tests for verbose mode rendering
    // =========================================================================

    #[test]
    fn snapshot_single_agent_verbose_with_prompt_and_tool_result() {
        let begin_event = SubAgentBeginEvent {
            call_id: "call-snap-1".to_string(),
            agent_type: "general".to_string(),
            description: "Echo and write to file".to_string(),
            prompt: "Please do the following two tasks:\n1. Echo \"Hello World\" using bash\n2. Write something to temp.txt".to_string(),
            sender_thread_id: ThreadId::default(),
            resumed: false,
        };
        let mut cell = new_subagent_cell(begin_event, false);

        if let Some(agent) = cell.find_agent_mut("call-snap-1") {
            // Add shell command with result
            agent.add_event(EventMsg::ExecCommandBegin(ExecCommandBeginEvent {
                call_id: "exec-1".to_string(),
                process_id: None,
                turn_id: "turn-1".to_string(),
                command: vec!["echo".to_string(), "Hello World".to_string()],
                cwd: PathBuf::from("/tmp"),
                parsed_cmd: vec![],
                source: ExecCommandSource::Agent,
                interaction_input: None,
            }));
            agent.add_event(EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                call_id: "exec-1".to_string(),
                process_id: None,
                turn_id: "turn-1".to_string(),
                command: vec!["echo".to_string(), "Hello World".to_string()],
                cwd: PathBuf::from("/tmp"),
                parsed_cmd: vec![],
                source: ExecCommandSource::Agent,
                interaction_input: None,
                stdout: "Hello World".to_string(),
                stderr: String::new(),
                aggregated_output: String::new(),
                exit_code: 0,
                duration: std::time::Duration::from_millis(50),
                formatted_output: "Hello World".to_string(),
            }));

            // Complete with response
            agent.complete_with_status(&AgentStatus::Completed(Some(
                "Task completed successfully.\n1. Echo output: Hello World\n2. File written."
                    .to_string(),
            )));
        }

        let ctx = RenderContext::with_verbosity(80, crate::verbosity::DisplayVerbosity::Verbose);
        let lines = cell.display_lines(ctx);
        let rendered = render_lines(&lines).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn snapshot_single_agent_verbose_with_multiline_output() {
        let begin_event = SubAgentBeginEvent {
            call_id: "call-snap-2".to_string(),
            agent_type: "Explore".to_string(),
            description: "Run multi-line command".to_string(),
            prompt: "Execute: echo 1; echo 2; echo 3".to_string(),
            sender_thread_id: ThreadId::default(),
            resumed: false,
        };
        let mut cell = new_subagent_cell(begin_event, false);

        if let Some(agent) = cell.find_agent_mut("call-snap-2") {
            agent.add_event(EventMsg::ExecCommandBegin(ExecCommandBeginEvent {
                call_id: "exec-1".to_string(),
                process_id: None,
                turn_id: "turn-1".to_string(),
                command: vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    "echo 1; echo 2; echo 3".to_string(),
                ],
                cwd: PathBuf::from("/tmp"),
                parsed_cmd: vec![],
                source: ExecCommandSource::Agent,
                interaction_input: None,
            }));
            agent.add_event(EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                call_id: "exec-1".to_string(),
                process_id: None,
                turn_id: "turn-1".to_string(),
                command: vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    "echo 1; echo 2; echo 3".to_string(),
                ],
                cwd: PathBuf::from("/tmp"),
                parsed_cmd: vec![],
                source: ExecCommandSource::Agent,
                interaction_input: None,
                stdout: "1\n2\n3".to_string(),
                stderr: String::new(),
                aggregated_output: String::new(),
                exit_code: 0,
                duration: std::time::Duration::from_millis(100),
                formatted_output: "1\n2\n3".to_string(),
            }));

            agent
                .complete_with_status(&AgentStatus::Completed(Some("Output: 1, 2, 3".to_string())));
        }

        let ctx = RenderContext::with_verbosity(80, crate::verbosity::DisplayVerbosity::Verbose);
        let lines = cell.display_lines(ctx);
        let rendered = render_lines(&lines).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn snapshot_single_agent_verbose_with_error() {
        let begin_event = SubAgentBeginEvent {
            call_id: "call-snap-3".to_string(),
            agent_type: "general".to_string(),
            description: "Run failing command".to_string(),
            prompt: "Try to read a non-existent file".to_string(),
            sender_thread_id: ThreadId::default(),
            resumed: false,
        };
        let mut cell = new_subagent_cell(begin_event, false);

        if let Some(agent) = cell.find_agent_mut("call-snap-3") {
            agent.add_event(EventMsg::ExecCommandBegin(ExecCommandBeginEvent {
                call_id: "exec-1".to_string(),
                process_id: None,
                turn_id: "turn-1".to_string(),
                command: vec!["cat".to_string(), "/nonexistent/file.txt".to_string()],
                cwd: PathBuf::from("/tmp"),
                parsed_cmd: vec![],
                source: ExecCommandSource::Agent,
                interaction_input: None,
            }));
            agent.add_event(EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                call_id: "exec-1".to_string(),
                process_id: None,
                turn_id: "turn-1".to_string(),
                command: vec!["cat".to_string(), "/nonexistent/file.txt".to_string()],
                cwd: PathBuf::from("/tmp"),
                parsed_cmd: vec![],
                source: ExecCommandSource::Agent,
                interaction_input: None,
                stdout: String::new(),
                stderr: "cat: /nonexistent/file.txt: No such file or directory".to_string(),
                aggregated_output: String::new(),
                exit_code: 1,
                duration: std::time::Duration::from_millis(10),
                formatted_output: String::new(),
            }));

            agent.complete_with_status(&AgentStatus::Errored("File not found".to_string()));
        }

        let ctx = RenderContext::with_verbosity(80, crate::verbosity::DisplayVerbosity::Verbose);
        let lines = cell.display_lines(ctx);
        let rendered = render_lines(&lines).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn snapshot_single_agent_compact_mode() {
        let begin_event = SubAgentBeginEvent {
            call_id: "call-snap-4".to_string(),
            agent_type: "Explore".to_string(),
            description: "Search codebase".to_string(),
            prompt: "Find all test files".to_string(),
            sender_thread_id: ThreadId::default(),
            resumed: false,
        };
        let mut cell = new_subagent_cell(begin_event, false);

        if let Some(agent) = cell.find_agent_mut("call-snap-4") {
            agent.add_event(EventMsg::ExecCommandBegin(ExecCommandBeginEvent {
                call_id: "exec-1".to_string(),
                process_id: None,
                turn_id: "turn-1".to_string(),
                command: vec![
                    "find".to_string(),
                    ".".to_string(),
                    "-name".to_string(),
                    "*_test.rs".to_string(),
                ],
                cwd: PathBuf::from("/project"),
                parsed_cmd: vec![],
                source: ExecCommandSource::Agent,
                interaction_input: None,
            }));
            agent.add_event(EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                call_id: "exec-1".to_string(),
                process_id: None,
                turn_id: "turn-1".to_string(),
                command: vec![
                    "find".to_string(),
                    ".".to_string(),
                    "-name".to_string(),
                    "*_test.rs".to_string(),
                ],
                cwd: PathBuf::from("/project"),
                parsed_cmd: vec![],
                source: ExecCommandSource::Agent,
                interaction_input: None,
                stdout: "./src/lib_test.rs\n./src/utils_test.rs".to_string(),
                stderr: String::new(),
                aggregated_output: String::new(),
                exit_code: 0,
                duration: std::time::Duration::from_millis(200),
                formatted_output: "./src/lib_test.rs\n./src/utils_test.rs".to_string(),
            }));

            agent.complete_with_status(&AgentStatus::Completed(Some(
                "Found 2 test files".to_string(),
            )));
        }

        // Compact mode (default)
        let lines = cell.display_lines(RenderContext::new(80));
        let rendered = render_lines(&lines).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn snapshot_agent_group_verbose() {
        let mut entry1 = SubAgentEntry::new(
            "call-g1".to_string(),
            "Explore".to_string(),
            "Search files".to_string(),
            Some("Find all Rust files".to_string()),
        );
        entry1.add_event(EventMsg::ExecCommandBegin(ExecCommandBeginEvent {
            call_id: "exec-g1".to_string(),
            process_id: None,
            turn_id: "turn-1".to_string(),
            command: vec![
                "find".to_string(),
                ".".to_string(),
                "-name".to_string(),
                "*.rs".to_string(),
            ],
            cwd: PathBuf::from("/project"),
            parsed_cmd: vec![],
            source: ExecCommandSource::Agent,
            interaction_input: None,
        }));
        entry1.add_event(EventMsg::ExecCommandEnd(ExecCommandEndEvent {
            call_id: "exec-g1".to_string(),
            process_id: None,
            turn_id: "turn-1".to_string(),
            command: vec![
                "find".to_string(),
                ".".to_string(),
                "-name".to_string(),
                "*.rs".to_string(),
            ],
            cwd: PathBuf::from("/project"),
            parsed_cmd: vec![],
            source: ExecCommandSource::Agent,
            interaction_input: None,
            stdout: "src/main.rs\nsrc/lib.rs".to_string(),
            stderr: String::new(),
            aggregated_output: String::new(),
            exit_code: 0,
            duration: std::time::Duration::from_millis(100),
            formatted_output: "src/main.rs\nsrc/lib.rs".to_string(),
        }));
        entry1.complete_with_status(&AgentStatus::Completed(Some("Found 2 files".to_string())));

        let mut entry2 = SubAgentEntry::new(
            "call-g2".to_string(),
            "Plan".to_string(),
            "Design solution".to_string(),
            Some("Create implementation plan".to_string()),
        );
        entry2.complete_with_status(&AgentStatus::Completed(Some("Plan created".to_string())));

        let cell = SubAgentCell::from_entries(vec![entry1, entry2], false);

        let ctx = RenderContext::with_verbosity(80, crate::verbosity::DisplayVerbosity::Verbose);
        let lines = cell.display_lines(ctx);
        let rendered = render_lines(&lines).join("\n");

        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn snapshot_agent_group_compact() {
        let mut entry1 = SubAgentEntry::new(
            "call-gc1".to_string(),
            "Explore".to_string(),
            "Search files".to_string(),
            None,
        );
        entry1.complete_with_status(&AgentStatus::Completed(None));

        let mut entry2 = SubAgentEntry::new(
            "call-gc2".to_string(),
            "Plan".to_string(),
            "Design solution".to_string(),
            None,
        );
        entry2.complete_with_status(&AgentStatus::Completed(None));

        let cell = SubAgentCell::from_entries(vec![entry1, entry2], false);

        // Compact mode
        let lines = cell.display_lines(RenderContext::new(80));
        let rendered = render_lines(&lines).join("\n");

        insta::assert_snapshot!(rendered);
    }
}
