use async_trait::async_trait;
use codex_protocol::permission_context::PermissionContext;

use crate::function_tool::FunctionCallError;
use crate::mcp_tool_call::handle_mcp_tool_call;
use crate::permissions::evaluate_rules_permission;
use crate::permissions::ToolPermissionResult;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

pub struct McpHandler;

#[async_trait]
impl ToolHandler for McpHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Mcp
    }

    async fn check_permissions(
        &self,
        invocation: &ToolInvocation,
        permission_context: &PermissionContext,
    ) -> ToolPermissionResult {
        // Extract MCP server and tool from payload
        let (server, tool) = match &invocation.payload {
            ToolPayload::Mcp { server, tool, .. } => (server.clone(), tool.clone()),
            _ => return ToolPermissionResult::passthrough(),
        };

        // Check MCP-specific rules: "mcp:server:tool" or just tool name
        let mcp_input = format!("{}:{}", server, tool);
        evaluate_rules_permission(permission_context, "Mcp", &mcp_input)
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            ..
        } = invocation;

        let payload = match payload {
            ToolPayload::Mcp {
                server,
                tool,
                raw_arguments,
            } => (server, tool, raw_arguments),
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "mcp handler received unsupported payload".to_string(),
                ));
            }
        };

        let (server, tool, raw_arguments) = payload;
        let arguments_str = raw_arguments;

        let response = handle_mcp_tool_call(
            session.as_ref(),
            turn.as_ref(),
            call_id.clone(),
            server,
            tool,
            arguments_str,
        )
        .await;

        match response {
            codex_protocol::models::ResponseInputItem::McpToolCallOutput { result, .. } => {
                Ok(ToolOutput::Mcp { result })
            }
            codex_protocol::models::ResponseInputItem::FunctionCallOutput { output, .. } => {
                let codex_protocol::models::FunctionCallOutputPayload {
                    content,
                    content_items,
                    success,
                } = output;
                Ok(ToolOutput::Function {
                    content,
                    content_items,
                    success,
                })
            }
            _ => Err(FunctionCallError::RespondToModel(
                "mcp handler received unexpected response variant".to_string(),
            )),
        }
    }
}
