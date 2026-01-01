use std::collections::BTreeMap;
use std::sync::LazyLock;

use async_trait::async_trait;
use codex_protocol::permission_context::PermissionContext;
use codex_protocol::protocol::EventMsg;
use codex_protocol::todo_tool::TodoWriteArgs;

use crate::client_common::tools::ResponsesApiTool;
use crate::client_common::tools::ToolSpec;
use crate::codex::Session;
use crate::codex::TurnContext;
use crate::function_tool::FunctionCallError;
use crate::permissions::ToolPermissionReason;
use crate::permissions::ToolPermissionResult;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::spec::JsonSchema;

pub struct TodoWriteHandler;

pub static TODO_WRITE_TOOL: LazyLock<ToolSpec> = LazyLock::new(|| {
    let mut todo_item_props = BTreeMap::new();
    todo_item_props.insert("step".to_string(), JsonSchema::String { description: None });
    todo_item_props.insert(
        "status".to_string(),
        JsonSchema::String {
            description: Some("One of: pending, in_progress, completed".to_string()),
        },
    );

    let todo_items_schema = JsonSchema::Array {
        description: Some("The list of TODO items".to_string()),
        items: Box::new(JsonSchema::Object {
            properties: todo_item_props,
            required: Some(vec!["step".to_string(), "status".to_string()]),
            additional_properties: Some(false.into()),
        }),
    };

    let mut properties = BTreeMap::new();
    properties.insert(
        "explanation".to_string(),
        JsonSchema::String { description: None },
    );
    properties.insert("todos".to_string(), todo_items_schema);

    ToolSpec::Function(ResponsesApiTool {
        name: "todo_write".to_string(),
        description: r#"Updates the TODO list for tracking task progress.
Provide an optional explanation and a list of TODO items, each with a step and status.
Use this tool frequently to track your progress and give the user visibility into what you're working on.
At most one step can be in_progress at a time.
"#
        .to_string(),
        strict: false,
        parameters: JsonSchema::Object {
            properties,
            required: Some(vec!["todos".to_string()]),
            additional_properties: Some(false.into()),
        },
    })
});

#[async_trait]
impl ToolHandler for TodoWriteHandler {
    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn check_permissions(
        &self,
        _invocation: &ToolInvocation,
        _permission_context: &PermissionContext,
    ) -> ToolPermissionResult {
        // TODO writes are always allowed (internal state management)
        ToolPermissionResult::allow(ToolPermissionReason::Safe)
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
                    "todo_write handler received unsupported payload".to_string(),
                ));
            }
        };

        let content =
            handle_todo_write(session.as_ref(), turn.as_ref(), arguments, call_id).await?;

        Ok(ToolOutput::Function {
            content,
            content_items: None,
            success: Some(true),
        })
    }
}

/// This function doesn't do anything useful. However, it gives the model a structured way to record its TODO list that clients can read and render.
/// So it's the _inputs_ to this function that are useful to clients, not the outputs and neither are actually useful for the model other
/// than forcing it to come up and document a TODO list (TBD how that affects performance).
pub(crate) async fn handle_todo_write(
    session: &Session,
    turn_context: &TurnContext,
    arguments: String,
    _call_id: String,
) -> Result<String, FunctionCallError> {
    let args = parse_todo_write_arguments(&arguments)?;
    session
        .send_event(turn_context, EventMsg::TodoUpdate(args.clone()))
        .await;

    // Persist TODO list to disk
    let codex_home = &session.services.codex_home;
    let session_id = session.conversation_id().to_string();
    let agent_id = session.source_session_id().map(String::as_str);
    if let Err(e) = crate::todos::save_todos(codex_home, &session_id, agent_id, &args) {
        tracing::warn!("Failed to persist TODO list: {}", e);
    }

    Ok("TODO list updated".to_string())
}

fn parse_todo_write_arguments(arguments: &str) -> Result<TodoWriteArgs, FunctionCallError> {
    serde_json::from_str::<TodoWriteArgs>(arguments).map_err(|e| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {e}"))
    })
}
