use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;

use crate::function_tool::FunctionCallError;
use crate::protocol::EventMsg;
use crate::protocol::MermaidToolCallEvent;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

pub struct MermaidHandler;

#[derive(Deserialize)]
struct MermaidArgs {
    code: String,
    #[serde(default)]
    citations: HashMap<String, String>,
}

#[async_trait]
impl ToolHandler for MermaidHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            call_id,
            payload,
            ..
        } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "mermaid handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: MermaidArgs = parse_arguments(&arguments)?;

        // Send the mermaid event to the frontend for rendering.
        session
            .send_event(
                turn.as_ref(),
                EventMsg::MermaidToolCall(MermaidToolCallEvent {
                    call_id,
                    code: args.code,
                    citations: args.citations,
                }),
            )
            .await;

        Ok(ToolOutput::Function {
            content: "<success>true</success>".to_string(),
            content_items: None,
            success: Some(true),
        })
    }
}
