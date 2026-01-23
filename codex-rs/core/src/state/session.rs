//! Session-wide mutable state.

use codex_protocol::models::ResponseItem;

use crate::codex::SessionConfiguration;
use crate::context_manager::ContextManager;
use crate::protocol::RateLimitSnapshot;
use crate::protocol::TokenUsage;
use crate::protocol::TokenUsageInfo;
use crate::read_file_state::ReadFileState;
use crate::truncate::TruncationPolicy;

/// Persistent, session-scoped state previously stored directly on `Session`.
pub(crate) struct SessionState {
    pub(crate) session_configuration: SessionConfiguration,
    pub(crate) history: ContextManager,
    pub(crate) latest_rate_limits: Option<RateLimitSnapshot>,
    pub(crate) server_reasoning_included: bool,

    // Plan mode state
    /// Plan slug for file naming (e.g., "tidy-popping-gizmo").
    /// Used to derive the plan file path via `resolve_plan_file_path()`.
    /// Path: `~/.codex/plans/{slug}.md`
    pub(crate) plan_slug: Option<String>,
    /// Whether this session is a plan mode subagent.
    pub(crate) is_subagent: bool,
    /// One-shot flag to emit exit attachment when leaving plan mode.
    pub(crate) needs_plan_exit_attachment: bool,

    /// Session-level file read tracking for edit validation and compaction recovery.
    /// Tracks which files have been read, their content, and modification time.
    pub(crate) read_file_state: ReadFileState,
}

impl SessionState {
    /// Create a new session state mirroring previous `State::default()` semantics.
    pub(crate) fn new(session_configuration: SessionConfiguration) -> Self {
        let history = ContextManager::new();
        Self {
            session_configuration,
            history,
            latest_rate_limits: None,
            server_reasoning_included: false,
            plan_slug: None,
            is_subagent: false,
            needs_plan_exit_attachment: false,
            read_file_state: ReadFileState::new(),
        }
    }

    // History helpers
    pub(crate) fn record_items<I>(&mut self, items: I, policy: TruncationPolicy)
    where
        I: IntoIterator,
        I::Item: std::ops::Deref<Target = ResponseItem>,
    {
        self.history.record_items(items, policy);
    }

    pub(crate) fn clone_history(&self) -> ContextManager {
        self.history.clone()
    }

    pub(crate) fn replace_history(&mut self, items: Vec<ResponseItem>) {
        self.history.replace(items);
    }

    pub(crate) fn set_token_info(&mut self, info: Option<TokenUsageInfo>) {
        self.history.set_token_info(info);
    }

    // Token/rate limit helpers
    pub(crate) fn update_token_info_from_usage(
        &mut self,
        usage: &TokenUsage,
        model_context_window: Option<i64>,
    ) {
        self.history.update_token_info(usage, model_context_window);
    }

    pub(crate) fn token_info(&self) -> Option<TokenUsageInfo> {
        self.history.token_info()
    }

    pub(crate) fn set_rate_limits(&mut self, snapshot: RateLimitSnapshot) {
        self.latest_rate_limits = Some(merge_rate_limit_fields(
            self.latest_rate_limits.as_ref(),
            snapshot,
        ));
    }

    pub(crate) fn token_info_and_rate_limits(
        &self,
    ) -> (Option<TokenUsageInfo>, Option<RateLimitSnapshot>) {
        (self.token_info(), self.latest_rate_limits.clone())
    }

    pub(crate) fn set_token_usage_full(&mut self, context_window: i64) {
        self.history.set_token_usage_full(context_window);
    }

    pub(crate) fn get_total_token_usage(&self, server_reasoning_included: bool) -> i64 {
        self.history
            .get_total_token_usage(server_reasoning_included)
    }

    pub(crate) fn set_server_reasoning_included(&mut self, included: bool) {
        self.server_reasoning_included = included;
    }

    pub(crate) fn server_reasoning_included(&self) -> bool {
        self.server_reasoning_included
    }

    // Plan mode helpers

    /// Set the plan slug and subagent flag when entering plan mode.
    pub(crate) fn set_plan_slug(&mut self, slug: String, is_subagent: bool) {
        self.plan_slug = Some(slug);
        self.is_subagent = is_subagent;
    }

    /// Get the current plan slug, or create one using the provided generator.
    pub(crate) fn get_or_create_plan_slug<F>(&mut self, generate: F) -> &str
    where
        F: FnOnce() -> String,
    {
        self.plan_slug.get_or_insert_with(generate)
    }

    /// Get the current plan slug if set.
    pub(crate) fn plan_slug(&self) -> Option<&str> {
        self.plan_slug.as_deref()
    }

    /// Check if this is a plan mode subagent.
    pub(crate) fn is_plan_subagent(&self) -> bool {
        self.is_subagent
    }

    /// Trigger the one-shot exit attachment flag (call when leaving plan mode).
    pub(crate) fn trigger_plan_exit_attachment(&mut self) {
        if self.plan_slug.is_some() {
            self.needs_plan_exit_attachment = true;
        }
    }

    /// Check if exit attachment is needed.
    pub(crate) fn needs_plan_exit_attachment(&self) -> bool {
        self.needs_plan_exit_attachment
    }

    /// Clear the one-shot exit attachment flag (call after emitting the attachment).
    pub(crate) fn clear_plan_exit_flag(&mut self) {
        self.needs_plan_exit_attachment = false;
    }
}

// Sometimes new snapshots don't include credits or plan information.
fn merge_rate_limit_fields(
    previous: Option<&RateLimitSnapshot>,
    mut snapshot: RateLimitSnapshot,
) -> RateLimitSnapshot {
    if snapshot.credits.is_none() {
        snapshot.credits = previous.and_then(|prior| prior.credits.clone());
    }
    if snapshot.plan_type.is_none() {
        snapshot.plan_type = previous.and_then(|prior| prior.plan_type);
    }
    snapshot
}
