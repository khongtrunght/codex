//! Turn-scoped state and active turn metadata scaffolding.

use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use codex_protocol::models::ResponseInputItem;
use codex_protocol::protocol::AskUserQuestionResponse;
use codex_protocol::protocol::ExitPlanModeApprovalResponse;
use tokio::sync::oneshot;

use crate::codex::TurnContext;
use crate::protocol::ReviewDecision;
use crate::tasks::SessionTask;

/// Metadata about the currently running turn.
pub(crate) struct ActiveTurn {
    pub(crate) tasks: IndexMap<String, RunningTask>,
    pub(crate) turn_state: Arc<Mutex<TurnState>>,
}

impl Default for ActiveTurn {
    fn default() -> Self {
        Self {
            tasks: IndexMap::new(),
            turn_state: Arc::new(Mutex::new(TurnState::default())),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TaskKind {
    Regular,
    Review,
    Compact,
    Enhance,
}

pub(crate) struct RunningTask {
    pub(crate) done: Arc<Notify>,
    pub(crate) kind: TaskKind,
    pub(crate) task: Arc<dyn SessionTask>,
    pub(crate) cancellation_token: CancellationToken,
    pub(crate) handle: Arc<AbortOnDropHandle<()>>,
    pub(crate) turn_context: Arc<TurnContext>,
    // Timer recorded when the task drops to capture the full turn duration.
    pub(crate) _timer: Option<codex_otel::Timer>,
}

impl ActiveTurn {
    pub(crate) fn add_task(&mut self, task: RunningTask) {
        let sub_id = task.turn_context.sub_id.clone();
        self.tasks.insert(sub_id, task);
    }

    pub(crate) fn remove_task(&mut self, sub_id: &str) -> bool {
        self.tasks.swap_remove(sub_id);
        self.tasks.is_empty()
    }

    pub(crate) fn drain_tasks(&mut self) -> Vec<RunningTask> {
        self.tasks.drain(..).map(|(_, task)| task).collect()
    }
}

/// Mutable state for a single turn.
#[derive(Default)]
pub(crate) struct TurnState {
    pending_approvals: HashMap<String, oneshot::Sender<ReviewDecision>>,
    pending_ask_user_question: HashMap<String, oneshot::Sender<AskUserQuestionResponse>>,
    pending_exit_plan_mode: HashMap<String, oneshot::Sender<ExitPlanModeApprovalResponse>>,
    pending_input: Vec<ResponseInputItem>,
}

impl TurnState {
    pub(crate) fn insert_pending_approval(
        &mut self,
        key: String,
        tx: oneshot::Sender<ReviewDecision>,
    ) -> Option<oneshot::Sender<ReviewDecision>> {
        self.pending_approvals.insert(key, tx)
    }

    pub(crate) fn remove_pending_approval(
        &mut self,
        key: &str,
    ) -> Option<oneshot::Sender<ReviewDecision>> {
        self.pending_approvals.remove(key)
    }

    pub(crate) fn clear_pending(&mut self) {
        self.pending_approvals.clear();
        self.pending_ask_user_question.clear();
        self.pending_exit_plan_mode.clear();
        self.pending_input.clear();
    }

    pub(crate) fn insert_pending_ask_user_question(
        &mut self,
        key: String,
        tx: oneshot::Sender<AskUserQuestionResponse>,
    ) -> Option<oneshot::Sender<AskUserQuestionResponse>> {
        self.pending_ask_user_question.insert(key, tx)
    }

    pub(crate) fn remove_pending_ask_user_question(
        &mut self,
        key: &str,
    ) -> Option<oneshot::Sender<AskUserQuestionResponse>> {
        self.pending_ask_user_question.remove(key)
    }

    pub(crate) fn insert_pending_exit_plan_mode(
        &mut self,
        key: String,
        tx: oneshot::Sender<ExitPlanModeApprovalResponse>,
    ) -> Option<oneshot::Sender<ExitPlanModeApprovalResponse>> {
        self.pending_exit_plan_mode.insert(key, tx)
    }

    pub(crate) fn remove_pending_exit_plan_mode(
        &mut self,
        key: &str,
    ) -> Option<oneshot::Sender<ExitPlanModeApprovalResponse>> {
        self.pending_exit_plan_mode.remove(key)
    }

    pub(crate) fn push_pending_input(&mut self, input: ResponseInputItem) {
        self.pending_input.push(input);
    }

    pub(crate) fn take_pending_input(&mut self) -> Vec<ResponseInputItem> {
        if self.pending_input.is_empty() {
            Vec::with_capacity(0)
        } else {
            let mut ret = Vec::new();
            std::mem::swap(&mut ret, &mut self.pending_input);
            ret
        }
    }

    pub(crate) fn has_pending_input(&self) -> bool {
        !self.pending_input.is_empty()
    }
}

impl ActiveTurn {
    /// Clear any pending approvals and input buffered for the current turn.
    pub(crate) async fn clear_pending(&self) {
        let mut ts = self.turn_state.lock().await;
        ts.clear_pending();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::config_types::CollaborationMode;

    #[test]
    fn turn_state_insert_and_remove_pending_exit_plan_mode() {
        let mut state = TurnState::default();
        let (tx, _rx) = oneshot::channel();

        // Insert returns None for new key
        let prev = state.insert_pending_exit_plan_mode("turn-1".to_string(), tx);
        assert!(prev.is_none());

        // Remove returns Some for existing key
        let removed = state.remove_pending_exit_plan_mode("turn-1");
        assert!(removed.is_some());

        // Remove returns None for non-existent key
        let removed_again = state.remove_pending_exit_plan_mode("turn-1");
        assert!(removed_again.is_none());
    }

    #[test]
    fn turn_state_overwrite_pending_exit_plan_mode() {
        let mut state = TurnState::default();
        let (tx1, _rx1) = oneshot::channel();
        let (tx2, _rx2) = oneshot::channel();

        // Insert first
        let prev1 = state.insert_pending_exit_plan_mode("turn-1".to_string(), tx1);
        assert!(prev1.is_none());

        // Insert second with same key returns first
        let prev2 = state.insert_pending_exit_plan_mode("turn-1".to_string(), tx2);
        assert!(prev2.is_some());
    }

    #[test]
    fn turn_state_clear_pending_clears_exit_plan_mode() {
        let mut state = TurnState::default();
        let (tx, _rx) = oneshot::channel();

        state.insert_pending_exit_plan_mode("turn-1".to_string(), tx);

        // Clear all pending
        state.clear_pending();

        // Should be gone
        let removed = state.remove_pending_exit_plan_mode("turn-1");
        assert!(removed.is_none());
    }

    #[tokio::test]
    async fn exit_plan_mode_approval_response_sent_through_channel() {
        let (tx, rx) = oneshot::channel();
        let mut state = TurnState::default();

        state.insert_pending_exit_plan_mode("turn-1".to_string(), tx);

        // Simulate removing and sending response
        let sender = state.remove_pending_exit_plan_mode("turn-1").unwrap();

        let response = ExitPlanModeApprovalResponse {
            approved: true,
            target_mode: Some(CollaborationMode::PairProgramming),
        };

        sender.send(response.clone()).unwrap();

        // Verify response received
        let received = rx.await.unwrap();
        assert!(received.approved);
        let mode = received.target_mode.unwrap();
        assert!(matches!(mode, CollaborationMode::PairProgramming));
    }

    #[tokio::test]
    async fn exit_plan_mode_approval_response_rejected() {
        let (tx, rx) = oneshot::channel();
        let mut state = TurnState::default();

        state.insert_pending_exit_plan_mode("turn-1".to_string(), tx);

        let sender = state.remove_pending_exit_plan_mode("turn-1").unwrap();

        let response = ExitPlanModeApprovalResponse {
            approved: false,
            target_mode: None,
        };

        sender.send(response).unwrap();

        let received = rx.await.unwrap();
        assert!(!received.approved);
        assert!(received.target_mode.is_none());
    }
}
