//! Session-wide mutable state.

use codex_protocol::models::ResponseItem;
use codex_protocol::session_mode::SessionMode;
use codex_protocol::session_mode_context::SessionModeContext;

use crate::codex::SessionConfiguration;
use crate::context_manager::ContextManager;
use crate::protocol::RateLimitSnapshot;
use crate::protocol::TokenUsage;
use crate::protocol::TokenUsageInfo;
use crate::truncate::TruncationPolicy;

/// Persistent, session-scoped state previously stored directly on `Session`.
pub(crate) struct SessionState {
    pub(crate) session_configuration: SessionConfiguration,
    pub(crate) history: ContextManager,
    pub(crate) latest_rate_limits: Option<RateLimitSnapshot>,
    /// Session mode context (workflow state, not permissions).
    pub(crate) mode_context: SessionModeContext,
}

impl SessionState {
    /// Create a new session state mirroring previous `State::default()` semantics.
    pub(crate) fn new(session_configuration: SessionConfiguration) -> Self {
        let history = ContextManager::new();
        Self {
            session_configuration,
            history,
            latest_rate_limits: None,
            mode_context: SessionModeContext::default(),
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

    pub(crate) fn get_total_token_usage(&self) -> i64 {
        self.history.get_total_token_usage()
    }

    // Mode context helpers

    /// Set the session mode.
    pub(crate) fn set_session_mode(&mut self, mode: SessionMode) {
        self.mode_context.set_mode(mode);
    }

    /// Enter plan mode with the given plan file path.
    pub(crate) fn enter_plan_mode(&mut self, plan_file_path: String) {
        self.mode_context.enter_plan_mode(plan_file_path);
    }

    /// Exit plan mode and return to default mode.
    pub(crate) fn exit_plan_mode(&mut self) {
        self.mode_context.exit_plan_mode();
    }

    /// Check if session mode is in planning state.
    pub(crate) fn is_planning(&self) -> bool {
        self.mode_context.is_planning()
    }

    /// Alias for is_planning() - check if in plan mode.
    pub(crate) fn is_in_plan_mode(&self) -> bool {
        self.is_planning()
    }

    /// Get the plan file path from mode context.
    pub(crate) fn plan_file_path(&self) -> Option<&str> {
        self.mode_context.plan_file_path()
    }

    /// Alias for plan_file_path() - get the plan file path.
    pub(crate) fn get_plan_file_path(&self) -> Option<&str> {
        self.plan_file_path()
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
