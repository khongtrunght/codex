use super::model::SubAgentCell;
use super::model::SubAgentGroupCell;
use super::model::SubAgentStatus;
use crate::exec_cell::spinner;
use crate::exec_command::strip_bash_lc_and_escape;
use crate::history_cell::HistoryCell;
use codex_core::protocol::EventMsg;
use codex_core::protocol::SubAgentBeginEvent;
use ratatui::prelude::*;

/// Create a new SubAgentCell from a begin event.
pub(crate) fn new_subagent_cell(
    begin_event: SubAgentBeginEvent,
    animations_enabled: bool,
) -> SubAgentCell {
    SubAgentCell::new(
        begin_event.call_id,
        begin_event.agent_type,
        begin_event.description,
        animations_enabled,
    )
}

/// Extracted tool call info for rendering.
#[derive(Debug, Clone)]
struct ToolCallInfo {
    tool_name: String,
    title: Option<String>,
}

/// Truncate string to n characters, adding "..." if truncated.
fn truncate_to_n_chars(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}...", &s[..n.saturating_sub(3)])
    }
}

/// Extract tool call info from raw events for display.
fn extract_tool_calls(raw_events: &[EventMsg]) -> Vec<ToolCallInfo> {
    let mut tool_calls = Vec::new();
    for event in raw_events {
        match event {
            EventMsg::ExecCommandBegin(begin) => {
                let cmd = strip_bash_lc_and_escape(&begin.command);
                tool_calls.push(ToolCallInfo {
                    tool_name: "shell".to_string(),
                    title: Some(cmd),
                });
            }
            EventMsg::McpToolCallBegin(begin) => {
                tool_calls.push(ToolCallInfo {
                    tool_name: begin.invocation.tool.clone(),
                    title: Some(begin.invocation.server.clone()),
                });
            }
            EventMsg::PatchApplyBegin(begin) => {
                let paths: Vec<String> = begin
                    .changes
                    .keys()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();
                let title = if paths.len() == 1 {
                    paths[0].clone()
                } else {
                    format!("{} files", paths.len())
                };
                tool_calls.push(ToolCallInfo {
                    tool_name: "patch".to_string(),
                    title: Some(title),
                });
            }
            EventMsg::WebSearchBegin(_) => {
                tool_calls.push(ToolCallInfo {
                    tool_name: "web_search".to_string(),
                    title: None,
                });
            }
            _ => {}
        }
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
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let status_icon = match &self.status {
            SubAgentStatus::Running => spinner(self.start_time, self.animations_enabled),
            SubAgentStatus::Completed(_) => "•".green().bold(),
            SubAgentStatus::Error(_) => "•".red().bold(),
        };

        let header: Line<'static> = vec![
            status_icon,
            " ".into(),
            self.agent_type.clone().bold(),
            "(".into(),
            self.description.clone().dim(),
            ")".into(),
        ]
        .into();

        let tool_calls = extract_tool_calls(&self.raw_events);
        let detail_line: Line<'static> = match &self.status {
            SubAgentStatus::Running => {
                let text = if let Some(last_call) = tool_calls.last() {
                    let display_arg = last_call.title.as_deref().unwrap_or("");
                    format!(
                        "  └ {}({})",
                        last_call.tool_name,
                        truncate_to_n_chars(display_arg, 60)
                    )
                } else {
                    "  └ Running...".to_string()
                };
                Line::from(text.dim())
            }
            SubAgentStatus::Completed(_) => {
                let tool_count = tool_calls.len();
                let tool_word = if tool_count == 1 {
                    "tool use"
                } else {
                    "tool uses"
                };
                let duration_str = self
                    .duration
                    .map(format_duration)
                    .unwrap_or_else(|| "?".to_string());
                Line::from(format!("  └ Done ({tool_count} {tool_word} · {duration_str})").dim())
            }
            SubAgentStatus::Error(msg) => {
                let duration_str = self
                    .duration
                    .map(format_duration)
                    .unwrap_or_else(|| "?".to_string());
                Line::from(format!("  └ Error: {msg} ({duration_str})").red())
            }
        };

        vec![header, detail_line]
    }

    fn transcript_animation_tick(&self) -> Option<u64> {
        if matches!(self.status, SubAgentStatus::Running) {
            self.start_time
                .map(|t| t.elapsed().as_millis() as u64 / 300)
        } else {
            None
        }
    }
}

impl HistoryCell for SubAgentGroupCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();

        for (i, cell) in self.cells.iter().enumerate() {
            let cell_lines = cell.display_lines(width);
            if i > 0 {
                // Add spacing between cells
                lines.push("".into());
            }
            lines.extend(cell_lines);
        }

        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_core::protocol::AgentStatus;
    use codex_core::protocol::ExecCommandBeginEvent;
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
        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines);

        // With animations disabled, spinner shows as "•"
        assert_eq!(rendered.len(), 2);
        assert_eq!(rendered[0], "• Explore(Find test files)");
        assert_eq!(rendered[1], "  └ Running...");
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

        // Add some tool events
        cell.add_event(EventMsg::ExecCommandBegin(ExecCommandBeginEvent {
            call_id: "exec-1".to_string(),
            process_id: None,
            turn_id: "turn-1".to_string(),
            command: vec!["ls".to_string(), "-la".to_string()],
            cwd: PathBuf::from("/tmp"),
            parsed_cmd: vec![],
            source: ExecCommandSource::Agent,
            interaction_input: None,
        }));
        cell.add_event(EventMsg::ExecCommandBegin(ExecCommandBeginEvent {
            call_id: "exec-2".to_string(),
            process_id: None,
            turn_id: "turn-1".to_string(),
            command: vec!["cat".to_string(), "file.txt".to_string()],
            cwd: PathBuf::from("/tmp"),
            parsed_cmd: vec![],
            source: ExecCommandSource::Agent,
            interaction_input: None,
        }));

        // Complete the cell
        cell.complete_with_status(&AgentStatus::Completed(None));

        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines);

        assert_eq!(rendered.len(), 2);
        assert_eq!(rendered[0], "• Plan(Design implementation)");
        // Should show "2 tool uses" and duration
        assert!(rendered[1].starts_with("  └ Done (2 tool uses · "));
    }

    #[test]
    fn subagent_cell_error_displays_in_red() {
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
        cell.complete_with_status(&AgentStatus::Errored("Something went wrong".to_string()));

        let lines = cell.display_lines(80);
        let rendered = render_lines(&lines);

        assert_eq!(rendered.len(), 2);
        assert_eq!(rendered[0], "• general(Process request)");
        // Error state shows error message
        assert!(rendered[1].starts_with("  └ Error: Something went wrong ("));
    }
}
