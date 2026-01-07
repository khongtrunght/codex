/*
Runtime: plan_mode

Implements Approvable trait for EnterPlanMode and ExitPlanMode tools.
These tools ALWAYS require user approval, even in bypass mode.
*/
use crate::tools::sandboxing::Approvable;
use crate::tools::sandboxing::ApprovalCtx;
use crate::tools::sandboxing::ExecApprovalRequirement;
use crate::tools::sandboxing::SandboxAttempt;
use crate::tools::sandboxing::Sandboxable;
use crate::tools::sandboxing::SandboxablePreference;
use crate::tools::sandboxing::ToolCtx;
use crate::tools::sandboxing::ToolError;
use crate::tools::sandboxing::ToolRuntime;
use crate::tools::sandboxing::with_cached_approval;
use codex_protocol::protocol::ReviewDecision;
use futures::future::BoxFuture;
use serde::Serialize;
use std::path::PathBuf;

/// Approval key for plan mode operations.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
pub struct PlanModeApprovalKey {
    /// "enter" or "exit"
    pub action: String,
    /// Session ID to scope approvals per session
    pub session_id: String,
}

/// Request to enter plan mode.
#[derive(Clone, Debug)]
pub struct EnterPlanModeRequest {
    pub session_id: String,
    /// Path to the plan file for approval display
    pub plan_file_path: PathBuf,
}

/// Output from entering plan mode (just a confirmation).
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct EnterPlanModeOutput {
    pub success: bool,
}

/// Runtime for EnterPlanMode tool.
#[derive(Default)]
pub struct EnterPlanModeRuntime;

impl EnterPlanModeRuntime {
    pub fn new() -> Self {
        Self
    }
}

impl Sandboxable for EnterPlanModeRuntime {
    fn sandbox_preference(&self) -> SandboxablePreference {
        // No sandbox needed for plan mode operations
        SandboxablePreference::Forbid
    }

    fn escalate_on_failure(&self) -> bool {
        false // Don't retry on failure
    }
}

impl Approvable<EnterPlanModeRequest> for EnterPlanModeRuntime {
    type ApprovalKey = PlanModeApprovalKey;

    fn approval_keys(&self, req: &EnterPlanModeRequest) -> Vec<Self::ApprovalKey> {
        vec![PlanModeApprovalKey {
            action: "enter".to_string(),
            session_id: req.session_id.clone(),
        }]
    }

    /// Plan mode tools ALWAYS need approval, regardless of policy.
    fn exec_approval_requirement(
        &self,
        _req: &EnterPlanModeRequest,
    ) -> Option<ExecApprovalRequirement> {
        Some(ExecApprovalRequirement::NeedsApproval {
            reason: Some("Enter plan mode?".to_string()),
            proposed_execpolicy_amendment: None,
        })
    }

    /// Force approval even in bypass mode.
    fn ignore_bypass_mode(&self) -> bool {
        true
    }

    fn start_approval_async<'a>(
        &'a mut self,
        req: &'a EnterPlanModeRequest,
        ctx: ApprovalCtx<'a>,
    ) -> BoxFuture<'a, ReviewDecision> {
        let keys = self.approval_keys(req);
        let session = ctx.session;
        let turn = ctx.turn;
        let call_id = ctx.call_id.to_string();
        let plan_file_path = req.plan_file_path.clone();

        Box::pin(async move {
            with_cached_approval(&session.services, keys, move || async move {
                session
                    .request_enter_plan_mode_approval(turn, call_id, plan_file_path)
                    .await
            })
            .await
        })
    }
}

impl ToolRuntime<EnterPlanModeRequest, EnterPlanModeOutput> for EnterPlanModeRuntime {
    async fn run(
        &mut self,
        _req: &EnterPlanModeRequest,
        _attempt: &SandboxAttempt<'_>,
        _ctx: &ToolCtx<'_>,
    ) -> Result<EnterPlanModeOutput, ToolError> {
        // The actual plan mode entry logic is handled by the handler
        // after approval. This runtime just confirms approval was granted.
        Ok(EnterPlanModeOutput { success: true })
    }
}

// ============================================================================
// ExitPlanMode Runtime
// ============================================================================

/// Request to exit plan mode.
#[derive(Clone, Debug)]
pub struct ExitPlanModeRequest {
    pub session_id: String,
    /// The plan content for display in approval UI
    pub plan_content: Option<String>,
    /// Path to the plan file for approval display
    pub plan_file_path: PathBuf,
}

/// Output from exiting plan mode.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ExitPlanModeOutput {
    pub success: bool,
}

/// Runtime for ExitPlanMode tool.
#[derive(Default)]
pub struct ExitPlanModeRuntime;

impl ExitPlanModeRuntime {
    pub fn new() -> Self {
        Self
    }
}

impl Sandboxable for ExitPlanModeRuntime {
    fn sandbox_preference(&self) -> SandboxablePreference {
        SandboxablePreference::Forbid
    }

    fn escalate_on_failure(&self) -> bool {
        false
    }
}

impl Approvable<ExitPlanModeRequest> for ExitPlanModeRuntime {
    type ApprovalKey = PlanModeApprovalKey;

    fn approval_keys(&self, req: &ExitPlanModeRequest) -> Vec<Self::ApprovalKey> {
        vec![PlanModeApprovalKey {
            action: "exit".to_string(),
            session_id: req.session_id.clone(),
        }]
    }

    fn exec_approval_requirement(
        &self,
        _req: &ExitPlanModeRequest,
    ) -> Option<ExecApprovalRequirement> {
        Some(ExecApprovalRequirement::NeedsApproval {
            reason: Some("Exit plan mode?".to_string()),
            proposed_execpolicy_amendment: None,
        })
    }

    /// Force approval even in bypass mode.
    fn ignore_bypass_mode(&self) -> bool {
        true
    }

    fn start_approval_async<'a>(
        &'a mut self,
        req: &'a ExitPlanModeRequest,
        ctx: ApprovalCtx<'a>,
    ) -> BoxFuture<'a, ReviewDecision> {
        let keys = self.approval_keys(req);
        let session = ctx.session;
        let turn = ctx.turn;
        let call_id = ctx.call_id.to_string();
        let plan_content = req.plan_content.clone().unwrap_or_default();
        let plan_file_path = req.plan_file_path.clone();

        Box::pin(async move {
            with_cached_approval(&session.services, keys, move || async move {
                session
                    .request_exit_plan_mode_approval(turn, call_id, plan_content, plan_file_path)
                    .await
            })
            .await
        })
    }
}

impl ToolRuntime<ExitPlanModeRequest, ExitPlanModeOutput> for ExitPlanModeRuntime {
    async fn run(
        &mut self,
        _req: &ExitPlanModeRequest,
        _attempt: &SandboxAttempt<'_>,
        _ctx: &ToolCtx<'_>,
    ) -> Result<ExitPlanModeOutput, ToolError> {
        Ok(ExitPlanModeOutput { success: true })
    }
}
