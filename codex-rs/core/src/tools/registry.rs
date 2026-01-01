use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::client_common::tools::ToolSpec;
use crate::function_tool::FunctionCallError;
use crate::permissions::PermissionBehavior;
use crate::permissions::ToolPermissionResult;
use crate::permissions::evaluate_mode_permission;
use crate::permissions::handle_dont_ask_mode;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use async_trait::async_trait;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::permission_context::PermissionContext;
use codex_utils_readiness::Readiness;
use tracing::warn;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolKind {
    Function,
    Mcp,
}

#[async_trait]
pub trait ToolHandler: Send + Sync {
    fn kind(&self) -> ToolKind;

    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(
            (self.kind(), payload),
            (ToolKind::Function, ToolPayload::Function { .. })
                | (ToolKind::Mcp, ToolPayload::Mcp { .. })
        )
    }

    async fn is_mutating(&self, _invocation: &ToolInvocation) -> bool {
        false
    }

    /// Tool-specific permission check (Claude Code pattern).
    ///
    /// Each tool implements its own permission logic, extracting relevant input
    /// from the invocation payload. This is the only permission check method -
    /// tools are responsible for:
    /// 1. Extracting their input (file_path, command, etc.)
    /// 2. Checking against rules if needed
    /// 3. Returning Allow/Deny/Ask/Passthrough
    ///
    /// # Arguments
    ///
    /// * `invocation` - The tool invocation context (contains parsed payload)
    /// * `permission_context` - Current permission context (mode, rules, etc.)
    ///
    /// # Returns
    ///
    /// * `Allow` - Proceed directly to execution
    /// * `Deny` - Return error immediately
    /// * `Ask` - Need user approval (will be converted to Deny in DontAsk mode)
    /// * `Passthrough` - Continue to next check (default behavior)
    async fn check_permissions(
        &self,
        _invocation: &ToolInvocation,
        _permission_context: &PermissionContext,
    ) -> ToolPermissionResult {
        // Default: passthrough (tool doesn't implement custom permission logic)
        ToolPermissionResult::passthrough()
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError>;
}

pub struct ToolRegistry {
    handlers: HashMap<String, Arc<dyn ToolHandler>>,
}

impl ToolRegistry {
    pub fn new(handlers: HashMap<String, Arc<dyn ToolHandler>>) -> Self {
        Self { handlers }
    }

    pub fn handler(&self, name: &str) -> Option<Arc<dyn ToolHandler>> {
        self.handlers.get(name).map(Arc::clone)
    }

    // TODO(jif) for dynamic tools.
    // pub fn register(&mut self, name: impl Into<String>, handler: Arc<dyn ToolHandler>) {
    //     let name = name.into();
    //     if self.handlers.insert(name.clone(), handler).is_some() {
    //         warn!("overwriting handler for tool {name}");
    //     }
    // }

    pub async fn dispatch(
        &self,
        invocation: ToolInvocation,
    ) -> Result<ResponseInputItem, FunctionCallError> {
        let tool_name = invocation.tool_name.clone();
        let call_id_owned = invocation.call_id.clone();
        let otel = invocation.turn.client.get_otel_manager();
        let payload_for_response = invocation.payload.clone();
        let log_payload = payload_for_response.log_payload();

        let handler = match self.handler(tool_name.as_ref()) {
            Some(handler) => handler,
            None => {
                let message =
                    unsupported_tool_call_message(&invocation.payload, tool_name.as_ref());
                otel.tool_result(
                    tool_name.as_ref(),
                    &call_id_owned,
                    log_payload.as_ref(),
                    Duration::ZERO,
                    false,
                    &message,
                );
                return Err(FunctionCallError::RespondToModel(message));
            }
        };

        if !handler.matches_kind(&invocation.payload) {
            let message = format!("tool {tool_name} invoked with incompatible payload");
            otel.tool_result(
                tool_name.as_ref(),
                &call_id_owned,
                log_payload.as_ref(),
                Duration::ZERO,
                false,
                &message,
            );
            return Err(FunctionCallError::Fatal(message));
        }

        // Permission check: Chained checks following Claude Code pattern
        // 1. Mode-based checks (BypassPermissions, AcceptEdits, etc.)
        // 2. Tool-specific checks (handler.check_permissions - extracts own input)
        // 3. DontAsk mode handling (converts Ask to Deny for subagents)
        let permission_context = invocation.session.get_permission_context().await;

        // Chain of permission checks - first non-passthrough result wins
        let perm_result = self
            .evaluate_permission_chain(&handler, &invocation, &permission_context, &tool_name)
            .await;

        // Handle the final permission result
        match perm_result.behavior {
            PermissionBehavior::Deny => {
                let message = perm_result
                    .message
                    .unwrap_or_else(|| format!("Permission denied for tool: {tool_name}"));
                otel.tool_result(
                    tool_name.as_ref(),
                    &call_id_owned,
                    log_payload.as_ref(),
                    Duration::ZERO,
                    false,
                    &message,
                );
                return Err(FunctionCallError::Denied(message));
            }
            PermissionBehavior::Allow => {
                // Proceed directly to execution
            }
            PermissionBehavior::Ask | PermissionBehavior::Passthrough => {
                // Fall through to existing tool-specific approval handling
                // This maintains backward compatibility with existing tests and approval flows.
                // The individual tool handlers (shell, apply_patch, etc.) will handle
                // their own approval logic via ToolOrchestrator or direct approval requests.
            }
        }

        let output_cell = tokio::sync::Mutex::new(None);

        let result = otel
            .log_tool_result(
                tool_name.as_ref(),
                &call_id_owned,
                log_payload.as_ref(),
                || {
                    let handler = handler.clone();
                    let output_cell = &output_cell;
                    let invocation = invocation;
                    async move {
                        if handler.is_mutating(&invocation).await {
                            tracing::trace!("waiting for tool gate");
                            invocation.turn.tool_call_gate.wait_ready().await;
                            tracing::trace!("tool gate released");
                        }
                        match handler.handle(invocation).await {
                            Ok(output) => {
                                let preview = output.log_preview();
                                let success = output.success_for_logging();
                                let mut guard = output_cell.lock().await;
                                *guard = Some(output);
                                Ok((preview, success))
                            }
                            Err(err) => Err(err),
                        }
                    }
                },
            )
            .await;

        match result {
            Ok(_) => {
                let mut guard = output_cell.lock().await;
                let output = guard.take().ok_or_else(|| {
                    FunctionCallError::Fatal("tool produced no output".to_string())
                })?;
                Ok(output.into_response(&call_id_owned, &payload_for_response))
            }
            Err(err) => Err(err),
        }
    }

    /// Evaluate the permission chain following Claude Code pattern.
    ///
    /// The chain is evaluated in order, and the first non-passthrough result wins:
    /// 1. Mode-based checks (BypassPermissions, AcceptEdits)
    /// 2. Tool-specific checks (handler.check_permissions - tool extracts its own input)
    /// 3. DontAsk mode handling (converts Ask to Deny for subagents)
    ///
    /// Note: Rule-based checks are handled WITHIN each tool's check_permissions().
    /// This is the Claude Code pattern - each tool knows its input structure.
    async fn evaluate_permission_chain(
        &self,
        handler: &Arc<dyn ToolHandler>,
        invocation: &ToolInvocation,
        permission_context: &PermissionContext,
        tool_name: &str,
    ) -> ToolPermissionResult {
        // 1. Mode-based checks (BypassPermissions, AcceptEdits)
        let mode_result = evaluate_mode_permission(permission_context, tool_name);
        if mode_result.stops_chain() {
            return handle_dont_ask_mode(permission_context, tool_name, mode_result);
        }

        // 2. Tool-specific checks (includes rule matching for that tool's input)
        let tool_result = handler
            .check_permissions(invocation, permission_context)
            .await;
        if tool_result.stops_chain() {
            return handle_dont_ask_mode(permission_context, tool_name, tool_result);
        }

        // 3. All checks passed through - default to passthrough
        // This allows existing tool-specific approval flows to continue
        ToolPermissionResult::passthrough()
    }
}

#[derive(Debug, Clone)]
pub struct ConfiguredToolSpec {
    pub spec: ToolSpec,
    pub supports_parallel_tool_calls: bool,
}

impl ConfiguredToolSpec {
    pub fn new(spec: ToolSpec, supports_parallel_tool_calls: bool) -> Self {
        Self {
            spec,
            supports_parallel_tool_calls,
        }
    }
}

pub struct ToolRegistryBuilder {
    handlers: HashMap<String, Arc<dyn ToolHandler>>,
    specs: Vec<ConfiguredToolSpec>,
}

impl ToolRegistryBuilder {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            specs: Vec::new(),
        }
    }

    pub fn push_spec(&mut self, spec: ToolSpec) {
        self.push_spec_with_parallel_support(spec, false);
    }

    pub fn push_spec_with_parallel_support(
        &mut self,
        spec: ToolSpec,
        supports_parallel_tool_calls: bool,
    ) {
        self.specs
            .push(ConfiguredToolSpec::new(spec, supports_parallel_tool_calls));
    }

    pub fn register_handler(&mut self, name: impl Into<String>, handler: Arc<dyn ToolHandler>) {
        let name = name.into();
        if self
            .handlers
            .insert(name.clone(), handler.clone())
            .is_some()
        {
            warn!("overwriting handler for tool {name}");
        }
    }

    // TODO(jif) for dynamic tools.
    // pub fn register_many<I>(&mut self, names: I, handler: Arc<dyn ToolHandler>)
    // where
    //     I: IntoIterator,
    //     I::Item: Into<String>,
    // {
    //     for name in names {
    //         let name = name.into();
    //         if self
    //             .handlers
    //             .insert(name.clone(), handler.clone())
    //             .is_some()
    //         {
    //             warn!("overwriting handler for tool {name}");
    //         }
    //     }
    // }

    /// Filter tools based on a sub-agent filter.
    ///
    /// This removes tool specs and handlers for tools that are not allowed
    /// according to the filter. Used to restrict tool access for sub-agents.
    pub fn apply_subagent_filter(&mut self, filter: &crate::tools::filtering::SubAgentToolFilter) {
        // Filter the specs
        self.specs.retain(|configured_spec| {
            let tool_name = configured_spec.spec.name();
            filter.is_tool_allowed(tool_name)
        });

        // Filter the handlers
        self.handlers.retain(|name, _| filter.is_tool_allowed(name));
    }

    pub fn build(self) -> (Vec<ConfiguredToolSpec>, ToolRegistry) {
        let registry = ToolRegistry::new(self.handlers);
        (self.specs, registry)
    }
}

fn unsupported_tool_call_message(payload: &ToolPayload, tool_name: &str) -> String {
    match payload {
        ToolPayload::Custom { .. } => format!("unsupported custom tool call: {tool_name}"),
        _ => format!("unsupported call: {tool_name}"),
    }
}
