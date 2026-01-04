//! Overlay for handling AskUserQuestion tool interactions.
//!
//! This module provides a TUI overlay that displays multiple-choice questions
//! to the user with support for:
//! - Tab navigation between questions
//! - Single-select (radio) and multi-select (checkbox) modes
//! - Custom "Other" text input option
//! - Submit/review screen

use std::collections::HashMap;
use std::collections::HashSet;

use codex_core::protocol::AskUserQuestion;
use codex_core::protocol::AskUserQuestionResponse;
use codex_core::protocol::Op;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use ratatui::widgets::Wrap;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::render::renderable::Renderable;

use super::CancellationEvent;
use super::bottom_pane_view::BottomPaneView;
use super::scroll_state::ScrollState;

// Unicode symbols for UI elements
// For multi-select option display
const MULTI_SELECT_ON: &str = "[✓]";
const MULTI_SELECT_OFF: &str = "[ ]";
// For tab bar question status
const TAB_CHECKBOX_ON: &str = "☒";
const TAB_CHECKBOX_OFF: &str = "☐";
// Other symbols
const TICK: &str = "✔";
const TICK_GREEN: &str = "✓";
const WARNING: &str = "⚠";
const BULLET: &str = "•";
const ARROW_RIGHT: &str = "→";

/// Index representing the submit/review screen (after all questions).
#[allow(dead_code)]
const SUBMIT_SCREEN_INDEX: usize = usize::MAX;

/// State for a single question.
#[derive(Debug, Default, Clone)]
struct QuestionState {
    /// Selected option index for single-select questions.
    selected_idx: Option<usize>,
    /// Selected option indices for multi-select questions.
    selected_indices: HashSet<usize>,
    /// Custom text input for "Other" option.
    custom_text: String,
    /// Whether the "Other" option is selected (for multi-select).
    other_selected: bool,
}

/// Modal overlay for asking user questions.
pub(crate) struct AskUserQuestionOverlay {
    /// Tool call ID to include in the response.
    call_id: String,
    /// The questions to ask.
    questions: Vec<AskUserQuestion>,
    /// Current question index (questions.len() = submit screen).
    current_question_idx: usize,
    /// Collected answers (question text -> answer string).
    answers: HashMap<String, String>,
    /// State for each question.
    question_states: HashMap<String, QuestionState>,
    /// Current selection within the options list.
    selection_idx: usize,
    /// Scroll state for options list.
    scroll_state: ScrollState,
    /// Whether we're in text input mode for "Other".
    is_in_text_input: bool,
    /// Whether the overlay is done (submitted or cancelled).
    done: bool,
    /// Event sender for emitting responses.
    app_event_tx: AppEventSender,
}

impl AskUserQuestionOverlay {
    /// Create a new overlay for the given questions.
    pub fn new(
        call_id: String,
        questions: Vec<AskUserQuestion>,
        app_event_tx: AppEventSender,
    ) -> Self {
        let mut question_states = HashMap::new();
        for q in &questions {
            question_states.insert(q.question.clone(), QuestionState::default());
        }

        Self {
            call_id,
            questions,
            current_question_idx: 0,
            answers: HashMap::new(),
            question_states,
            selection_idx: 0,
            scroll_state: ScrollState::new(),
            is_in_text_input: false,
            done: false,
            app_event_tx,
        }
    }

    /// Get the current question, or None if on submit screen.
    fn current_question(&self) -> Option<&AskUserQuestion> {
        self.questions.get(self.current_question_idx)
    }

    /// Check if currently on the submit/review screen.
    fn is_on_submit_screen(&self) -> bool {
        self.current_question_idx >= self.questions.len()
    }

    /// Total number of options for the current question (including "Other" and "Next" for multi-select).
    fn option_count(&self) -> usize {
        self.current_question()
            .map(|q| {
                if q.multi_select {
                    q.options.len() + 2 // +1 for "Other", +1 for "Next"
                } else {
                    q.options.len() + 1 // +1 for "Other"
                }
            })
            .unwrap_or(2) // Submit screen has 2 options
    }

    /// Navigate to previous question.
    fn prev_question(&mut self) {
        if self.current_question_idx > 0 {
            // Auto-save multi-select answer before navigating away
            self.auto_save_current_question();
            self.current_question_idx -= 1;
            self.reset_selection_for_current_question();
        }
    }

    /// Navigate to next question.
    fn next_question(&mut self) {
        // For single question without submit screen, don't advance
        if self.questions.len() == 1 && !self.is_on_submit_screen() {
            return;
        }
        if self.current_question_idx < self.questions.len() {
            // Auto-save multi-select answer before navigating away
            self.auto_save_current_question();
            self.current_question_idx += 1;
            self.reset_selection_for_current_question();
        }
    }

    /// Auto-save the current question's answer if it's multi-select with selections.
    fn auto_save_current_question(&mut self) {
        if let Some(q) = self.current_question().cloned() {
            if q.multi_select {
                self.save_multi_select_answer(&q);
            }
        }
    }

    /// Reset selection state when navigating to a new question.
    fn reset_selection_for_current_question(&mut self) {
        self.selection_idx = 0;
        self.scroll_state = ScrollState::new();
        self.is_in_text_input = false;
    }

    /// Move selection up.
    fn move_up(&mut self) {
        if self.selection_idx > 0 {
            self.selection_idx -= 1;
        } else {
            // Wrap to bottom
            self.selection_idx = self.option_count().saturating_sub(1);
        }
        self.update_text_input_mode();
    }

    /// Move selection down.
    fn move_down(&mut self) {
        let count = self.option_count();
        if self.selection_idx < count.saturating_sub(1) {
            self.selection_idx += 1;
        } else {
            // Wrap to top
            self.selection_idx = 0;
        }
        self.update_text_input_mode();
    }

    /// Update whether we're in text input mode based on selection.
    fn update_text_input_mode(&mut self) {
        if let Some(q) = self.current_question() {
            // "Other/Type something" option is at q.options.len()
            // For multi-select, "Next" is at q.options.len() + 1
            let other_idx = q.options.len();
            self.is_in_text_input = self.selection_idx == other_idx;
        } else {
            self.is_in_text_input = false;
        }
    }

    /// Select option by number key (1-9).
    fn select_by_number(&mut self, c: char) {
        if let Some(digit) = c.to_digit(10) {
            let idx = (digit as usize).saturating_sub(1);
            if idx < self.option_count() {
                self.selection_idx = idx;
                self.update_text_input_mode();
                if !self.is_in_text_input {
                    self.accept_selection();
                }
            }
        }
    }

    /// Accept the current selection.
    fn accept_selection(&mut self) {
        if self.is_on_submit_screen() {
            self.handle_submit_selection();
        } else if let Some(q) = self.current_question().cloned() {
            if q.multi_select {
                self.handle_multi_select_accept(&q);
            } else {
                self.handle_single_select_accept(&q);
            }
        }
    }

    /// Handle selection acceptance for single-select questions.
    fn handle_single_select_accept(&mut self, q: &AskUserQuestion) {
        let state = self.question_states.entry(q.question.clone()).or_default();
        let is_other = self.selection_idx == q.options.len();

        if is_other {
            if self.is_in_text_input {
                // Use custom text if provided
                if !state.custom_text.is_empty() {
                    self.answers.insert(q.question.clone(), state.custom_text.clone());
                    self.is_in_text_input = false;
                    self.auto_advance();
                }
                // If empty, stay in text input mode (user needs to type something)
            } else {
                // Enter text input mode
                self.is_in_text_input = true;
            }
        } else if let Some(opt) = q.options.get(self.selection_idx) {
            state.selected_idx = Some(self.selection_idx);
            self.answers.insert(q.question.clone(), opt.label.clone());
            self.auto_advance();
        }
    }

    /// Handle selection acceptance for multi-select questions.
    fn handle_multi_select_accept(&mut self, q: &AskUserQuestion) {
        let other_idx = q.options.len();
        let next_idx = q.options.len() + 1;

        // If "Next" is selected, save answer and advance
        if self.selection_idx == next_idx {
            self.save_multi_select_answer(q);
            self.auto_advance();
            return;
        }

        // If on "Type something"
        if self.selection_idx == other_idx {
            if self.is_in_text_input {
                // Confirm custom text and move to Next
                let state = self.question_states.entry(q.question.clone()).or_default();
                if !state.custom_text.is_empty() {
                    state.other_selected = true;
                }
                // Exit text input mode and move to Next
                self.is_in_text_input = false;
                self.selection_idx = next_idx;
            } else {
                // Enter text input mode
                self.is_in_text_input = true;
            }
            return;
        }

        // Otherwise toggle the current selection
        self.toggle_selection();
    }

    /// Save the multi-select answer to the answers map.
    fn save_multi_select_answer(&mut self, q: &AskUserQuestion) {
        let state = self.question_states.entry(q.question.clone()).or_default();

        // Build answer from selected options
        let mut selected_labels: Vec<String> = state
            .selected_indices
            .iter()
            .filter_map(|&idx| q.options.get(idx).map(|o| o.label.clone()))
            .collect();

        // Add custom text if "Other" is selected
        if state.other_selected && !state.custom_text.is_empty() {
            selected_labels.push(state.custom_text.clone());
        }

        if !selected_labels.is_empty() {
            self.answers.insert(q.question.clone(), selected_labels.join(", "));
        }
    }

    /// Toggle selection for multi-select questions.
    fn toggle_selection(&mut self) {
        if let Some(q) = self.current_question().cloned() {
            if !q.multi_select {
                return;
            }

            let state = self.question_states.entry(q.question.clone()).or_default();
            let is_other = self.selection_idx == q.options.len();

            if is_other {
                state.other_selected = !state.other_selected;
            } else if state.selected_indices.contains(&self.selection_idx) {
                state.selected_indices.remove(&self.selection_idx);
            } else {
                state.selected_indices.insert(self.selection_idx);
            }
        }
    }

    /// Auto-advance to next question or submit screen.
    fn auto_advance(&mut self) {
        if self.questions.len() == 1 {
            // Single question: submit immediately
            self.submit();
        } else {
            self.next_question();
        }
    }

    /// Handle submit screen selection.
    fn handle_submit_selection(&mut self) {
        if self.selection_idx == 0 {
            // Submit
            self.submit();
        } else {
            // Cancel
            self.cancel();
        }
    }

    /// Handle text input character.
    fn handle_text_input(&mut self, c: char) {
        if let Some(q) = self.current_question() {
            let state = self.question_states.entry(q.question.clone()).or_default();
            state.custom_text.push(c);
        }
    }

    /// Handle backspace in text input.
    fn handle_backspace(&mut self) {
        if let Some(q) = self.current_question() {
            let state = self.question_states.entry(q.question.clone()).or_default();
            state.custom_text.pop();
        }
    }

    /// Exit text input mode without cancelling the entire dialog.
    fn exit_text_input_mode(&mut self) {
        if let Some(q) = self.current_question() {
            let state = self.question_states.entry(q.question.clone()).or_default();
            // Clear the custom text
            state.custom_text.clear();
            state.other_selected = false;
        }
        self.is_in_text_input = false;
    }

    /// Submit the answers.
    fn submit(&mut self) {
        self.send_response(false);
        self.done = true;
    }

    /// Cancel and close the overlay, interrupting the current turn.
    fn cancel(&mut self) {
        self.send_response(true);
        // Also send interrupt to stop the model from continuing
        self.app_event_tx.send(AppEvent::CodexOp(Op::Interrupt));
        self.done = true;
    }

    /// Send the response back to the core.
    fn send_response(&self, cancelled: bool) {
        let response = AskUserQuestionResponse {
            answers: self.answers.clone(),
            cancelled,
        };
        self.app_event_tx.send(AppEvent::CodexOp(
            Op::ResolveAskUserQuestion {
                call_id: self.call_id.clone(),
                response,
            },
        ));
    }

    /// Check if all questions have been answered.
    fn all_questions_answered(&self) -> bool {
        self.questions
            .iter()
            .all(|q| self.answers.contains_key(&q.question))
    }

    /// Build the tab bar line.
    fn build_tab_bar(&self, _width: u16) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = Vec::new();

        // Left arrow
        let can_go_left = self.current_question_idx > 0;
        spans.push(if can_go_left {
            Span::raw("\u{2190} ")
        } else {
            Span::raw("\u{2190} ").dim()
        });

        // Question tabs
        for (idx, q) in self.questions.iter().enumerate() {
            let is_current = idx == self.current_question_idx;
            let is_answered = self.answers.contains_key(&q.question);
            let check = if is_answered {
                TAB_CHECKBOX_ON
            } else {
                TAB_CHECKBOX_OFF
            };

            // Truncate header if needed
            let header = truncate_string(&q.header, 12);
            let tab_text = format!(" {} {} ", check, header);

            if is_current {
                // Light background with black text for focused tab
                spans.push(Span::raw(tab_text).on_light_blue().black());
            } else {
                spans.push(Span::raw(tab_text));
            }
        }

        // Submit tab (only for multiple questions)
        if self.questions.len() > 1 {
            let is_current = self.is_on_submit_screen();
            let tab_text = format!(" {} Submit ", TICK);

            if is_current {
                spans.push(Span::raw(tab_text).on_light_blue().black());
            } else {
                spans.push(Span::raw(tab_text));
            }
        }

        // Right arrow
        let can_go_right = self.current_question_idx < self.questions.len();
        spans.push(if can_go_right {
            Span::raw(" \u{2192}")
        } else {
            Span::raw(" \u{2192}").dim()
        });

        Line::from(spans)
    }

    /// Build option lines for the current question.
    fn build_question_options(&self) -> Vec<Line<'static>> {
        let Some(q) = self.current_question() else {
            return Vec::new();
        };

        let state = self.question_states.get(&q.question);
        let mut lines = Vec::new();

        // Check if we're in text input mode (typing in "Type something")
        let other_idx = q.options.len();
        let is_typing_custom = self.is_in_text_input && self.selection_idx == other_idx;

        // Check if this question already has an answer (for showing green tick)
        let existing_answer = self.answers.get(&q.question);

        for (idx, opt) in q.options.iter().enumerate() {
            let is_focused = self.selection_idx == idx;
            let num = idx + 1;
            let prefix = if is_focused { "\u{203A}" } else { " " };

            // Check if this option was the previous answer (for single-select)
            let is_previous_answer = !q.multi_select
                && existing_answer
                    .map(|a| a == &opt.label)
                    .unwrap_or(false);

            // Check if this option is checked (for multi-select)
            let is_checked = q.multi_select
                && state
                    .map(|s| s.selected_indices.contains(&idx))
                    .unwrap_or(false);

            if q.multi_select {
                // Multi-select: show checkbox with proper colors
                let check_mark = if is_checked {
                    MULTI_SELECT_ON
                } else {
                    MULTI_SELECT_OFF
                };

                // Build spans with proper coloring
                let mut spans = Vec::new();
                spans.push(Span::raw(format!("{} {}. ", prefix, num)));

                // Checkbox: green if checked
                if is_checked {
                    spans.push(Span::raw(check_mark).green());
                    spans.push(Span::raw(format!(" {}", opt.label)).green());
                } else if is_focused {
                    spans.push(Span::raw(check_mark));
                    spans.push(Span::raw(format!(" {}", opt.label)).light_blue());
                } else {
                    spans.push(Span::raw(format!("{} {}", check_mark, opt.label)));
                }

                let line = Line::from(spans);
                if is_focused {
                    lines.push(line.bold());
                } else {
                    lines.push(line);
                }
            } else if is_previous_answer {
                // Single-select with previous answer: show green tick
                let label_line = format!("{} {}. {} {}", prefix, num, opt.label, TICK_GREEN);
                lines.push(Line::from(label_line).green());
            } else if is_focused {
                // Single-select focused: blue color
                let label_line = format!("{} {}. {}", prefix, num, opt.label);
                lines.push(Line::from(label_line).light_blue().bold());
            } else {
                // Single-select: no checkbox/radio, just number and label
                let label_line = format!("{} {}. {}", prefix, num, opt.label);
                lines.push(Line::from(label_line));
            }

            // Always show description for all options
            if !opt.description.is_empty() {
                let desc_line = Line::from(format!("   {}", opt.description)).dim();
                if is_checked {
                    lines.push(desc_line.green());
                } else if is_focused {
                    lines.push(desc_line.light_blue());
                } else {
                    lines.push(desc_line);
                }
            }
        }

        // "Type something" option
        let is_other_selected = self.selection_idx == other_idx;
        let custom_text = state.map(|s| s.custom_text.as_str()).unwrap_or("");
        let num = other_idx + 1;
        let prefix = if is_other_selected { "\u{203A}" } else { " " };

        // Check if custom text was the previous answer (for single-select)
        let custom_was_answer = !q.multi_select
            && existing_answer
                .map(|a| {
                    // Answer is custom if it doesn't match any option label
                    !q.options.iter().any(|o| &o.label == a)
                })
                .unwrap_or(false);

        if is_typing_custom {
            // When in text input mode, show typed text with cursor (no radio button)
            let display_text = format!("{} {}. {}", prefix, num, custom_text);

            // Show with cursor block at end
            lines.push(Line::from(vec![
                Span::raw(display_text).bold(),
                Span::raw("\u{2588}").on_light_blue(), // Block cursor
            ]));
        } else if custom_was_answer {
            // Show the previous custom answer with green tick
            let answer_text = existing_answer.cloned().unwrap_or_default();
            let other_line = format!("{} {}. {} {}", prefix, num, answer_text, TICK_GREEN);
            lines.push(Line::from(other_line).green());
        } else {
            // Normal display
            let other_label = if custom_text.is_empty() {
                "Type something".to_string()
            } else {
                custom_text.to_string()
            };

            if q.multi_select {
                // Multi-select: show checkbox with proper colors
                let is_other_checked = state.map(|s| s.other_selected).unwrap_or(false);
                let check_mark = if is_other_checked {
                    MULTI_SELECT_ON
                } else {
                    MULTI_SELECT_OFF
                };

                let mut spans = Vec::new();
                spans.push(Span::raw(format!("{} {}. ", prefix, num)));

                if is_other_checked {
                    spans.push(Span::raw(check_mark).green());
                    spans.push(Span::raw(format!(" {}", other_label)).green());
                } else if is_other_selected {
                    spans.push(Span::raw(check_mark));
                    spans.push(Span::raw(format!(" {}", other_label)).light_blue());
                } else {
                    spans.push(Span::raw(format!("{} {}", check_mark, other_label)));
                }

                let line = Line::from(spans);
                if is_other_selected {
                    lines.push(line.bold());
                } else {
                    lines.push(line);
                }
            } else {
                // Single-select: no checkbox
                let other_line = format!("{} {}. {}", prefix, num, other_label);

                if is_other_selected {
                    lines.push(Line::from(other_line).light_blue().bold());
                } else {
                    lines.push(Line::from(other_line));
                }
            }
        }

        // "Next/Submit" option for multi-select questions
        if q.multi_select {
            let next_idx = other_idx + 1;
            let is_next_selected = self.selection_idx == next_idx;
            let prefix = if is_next_selected { "\u{203A}" } else { " " };

            // Show "Submit" on last question, "Next" otherwise
            let is_last_question = self.current_question_idx == self.questions.len() - 1;
            let button_text = if is_last_question { "Submit" } else { "Next" };
            let next_line = format!("{}  {}", prefix, button_text);

            if is_next_selected {
                lines.push(Line::from(next_line).light_blue().bold());
            } else {
                lines.push(Line::from(next_line));
            }
        }

        lines
    }

    /// Build the submit/review screen content.
    fn build_submit_screen(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        lines.push(Line::from("Review your answers").bold());
        lines.push(Line::from(""));

        // Warning if not all answered
        if !self.all_questions_answered() {
            lines.push(
                Line::from(vec![
                    Span::raw(WARNING).yellow(),
                    Span::raw(" You have not answered all questions").yellow(),
                ])
            );
            lines.push(Line::from(""));
        }

        // List answers
        for q in &self.questions {
            if let Some(answer) = self.answers.get(&q.question) {
                lines.push(Line::from(vec![
                    Span::raw(BULLET),
                    Span::raw(" "),
                    Span::raw(q.question.clone()),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::raw(ARROW_RIGHT).green(),
                    Span::raw(" "),
                    Span::raw(answer.clone()).green(),
                ]));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from("Ready to submit your answers?").dim());
        lines.push(Line::from(""));

        // Submit/Cancel options
        let submit_prefix = if self.selection_idx == 0 { "\u{203A}" } else { " " };
        let cancel_prefix = if self.selection_idx == 1 { "\u{203A}" } else { " " };

        let submit_line = format!("{} 1. Submit answers", submit_prefix);
        let cancel_line = format!("{} 2. Cancel", cancel_prefix);

        if self.selection_idx == 0 {
            lines.push(Line::from(submit_line).bold());
        } else {
            lines.push(Line::from(submit_line));
        }

        if self.selection_idx == 1 {
            lines.push(Line::from(cancel_line).bold());
        } else {
            lines.push(Line::from(cancel_line));
        }

        lines
    }

    /// Build the footer hint line.
    fn build_footer(&self) -> Line<'static> {
        Line::from("Enter to select \u{00B7} Tab/Arrow keys to navigate \u{00B7} Esc to cancel").dim()
    }
}

impl BottomPaneView for AskUserQuestionOverlay {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event {
            // Tab navigation (Left/Right or Shift+Tab/Tab)
            KeyEvent {
                code: KeyCode::Left,
                ..
            }
            | KeyEvent {
                code: KeyCode::BackTab,
                ..
            } if !self.is_in_text_input => {
                self.prev_question();
            }

            KeyEvent {
                code: KeyCode::Right,
                ..
            } if !self.is_in_text_input => {
                self.next_question();
            }

            KeyEvent {
                code: KeyCode::Tab,
                modifiers: KeyModifiers::NONE,
                ..
            } if !self.is_in_text_input => {
                self.next_question();
            }

            // Option navigation (also works in text input to exit)
            KeyEvent {
                code: KeyCode::Up, ..
            }
            | KeyEvent {
                code: KeyCode::Char('k'),
                modifiers: KeyModifiers::NONE,
                ..
            } if !self.is_in_text_input || key_event.code == KeyCode::Up => {
                // Exit text input mode if active
                if self.is_in_text_input {
                    self.is_in_text_input = false;
                }
                self.move_up();
            }

            KeyEvent {
                code: KeyCode::Down,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('j'),
                modifiers: KeyModifiers::NONE,
                ..
            } if !self.is_in_text_input || key_event.code == KeyCode::Down => {
                // Exit text input mode if active
                if self.is_in_text_input {
                    self.is_in_text_input = false;
                }
                self.move_down();
            }

            // Selection
            KeyEvent {
                code: KeyCode::Enter,
                ..
            } => {
                self.accept_selection();
            }

            // Toggle for multi-select
            KeyEvent {
                code: KeyCode::Char(' '),
                ..
            } if !self.is_in_text_input => {
                if let Some(q) = self.current_question() {
                    if q.multi_select {
                        self.toggle_selection();
                    }
                }
            }

            // ESC - Exit text input mode if typing, otherwise cancel entire dialog
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                if self.is_in_text_input {
                    self.exit_text_input_mode();
                } else {
                    self.cancel();
                }
            }

            // Number shortcuts (1-9)
            KeyEvent {
                code: KeyCode::Char(c @ '1'..='9'),
                modifiers: KeyModifiers::NONE,
                ..
            } if !self.is_in_text_input => {
                self.select_by_number(c);
            }

            // Text input handling
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers,
                ..
            } if self.is_in_text_input
                && !modifiers.contains(KeyModifiers::CONTROL)
                && !modifiers.contains(KeyModifiers::ALT) =>
            {
                self.handle_text_input(c);
            }

            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } if self.is_in_text_input => {
                self.handle_backspace();
            }

            _ => {}
        }
    }

    fn is_complete(&self) -> bool {
        self.done
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.cancel();
        CancellationEvent::Handled
    }
}

impl Renderable for AskUserQuestionOverlay {
    fn desired_height(&self, _width: u16) -> u16 {
        // Tab bar + question/content + footer
        let tab_bar_height = 1;
        let footer_height = 1;
        let padding = 2;

        let content_height = if self.is_on_submit_screen() {
            // Submit screen: title + warning + answers + options
            let answer_lines = self.questions.len() * 2 + 6;
            answer_lines as u16
        } else {
            // Question screen: title + options
            let option_count = self.option_count();
            let description_lines = option_count; // Assume worst case
            (3 + option_count * 2 + description_lines) as u16
        };

        tab_bar_height + content_height + footer_height + padding
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        // Layout: tab bar, content, footer
        let [tab_area, content_area, footer_area] = Layout::vertical([
            Constraint::Length(2), // Tab bar + margin
            Constraint::Fill(1),   // Content
            Constraint::Length(1), // Footer
        ])
        .areas(area);

        // Render tab bar
        let tab_bar = self.build_tab_bar(tab_area.width);
        Paragraph::new(tab_bar).render(tab_area, buf);

        // Render content
        let content_lines = if self.is_on_submit_screen() {
            self.build_submit_screen()
        } else {
            let mut lines = Vec::new();

            // Question title
            if let Some(q) = self.current_question() {
                lines.push(Line::from(q.question.clone()).bold());
                lines.push(Line::from(""));
            }

            // Options
            lines.extend(self.build_question_options());

            lines
        };

        let content = Paragraph::new(content_lines).wrap(Wrap { trim: false });
        content.render(content_area, buf);

        // Render footer
        let footer = self.build_footer();
        Paragraph::new(footer).render(footer_area, buf);
    }
}

/// Truncate a string to max_len characters, adding "..." if truncated.
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(1)).collect();
        format!("{}\u{2026}", truncated)
    }
}
