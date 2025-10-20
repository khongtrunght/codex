# Anthropic Support for Codex-RS Implementation Plan

## Overview

This plan details the implementation of Anthropic Claude model support in codex-rs, enabling users to leverage Claude models (3.5 Sonnet, Opus, Haiku) as an alternative to OpenAI's GPT models while maintaining full backward compatibility and the existing user experience.

## Current State Analysis

Based on thorough codebase analysis at commit 4f46360a, the integration requires:
- Adding a new `WireApi::Anthropic` variant (only 2 pattern matches to update)
- Creating separate file editing tools (WriteFile, EditFile, DeleteFile don't exist)
- Extending authentication to support Anthropic API keys and OAuth
- Implementing SSE streaming for Anthropic's named event format
- Adding tool conversion from Codex format to Anthropic's `input_schema` format

### Key Discoveries:
- The `WireApi` enum at `core/src/model_provider_info.rs:31-40` has only two variants
- Pattern matches exist at `core/src/client.rs:139` and `core/src/model_provider_info.rs:159`
- SSE parsing already uses `eventsource_stream` which supports named events
- File editing tools need to be created from scratch (only ReadFile exists)
- The specifications contain inaccuracies that must be corrected first

## Desired End State

After implementation:
- Users can configure Anthropic as their model provider via environment variables or config
- Claude models work seamlessly with existing Codex commands and workflows
- Tool calling, file editing, and streaming work correctly with Claude
- Both API key and OAuth authentication are supported
- OpenAI integration remains completely unaffected

### Verification Criteria:
- Claude models can be invoked via `codex --model claude-3-5-sonnet-20241022 --provider anthropic`
- Streaming responses display correctly in the TUI
- Tool calling executes successfully with proper result handling
- File operations work effectively with Claude's separate tools approach
- OAuth flow completes successfully for Claude Code users

## What We're NOT Doing

- Amazon Bedrock or Google Vertex AI integration (future enhancement)
- Vision/image input capabilities (deferred)
- Prompt caching features (future optimization)
- Computer use capabilities (experimental, out of scope)
- Breaking changes to existing OpenAI integration
- TUI redesign for Anthropic-specific features

## Implementation Approach

We'll implement Anthropic support incrementally, following the existing provider abstraction patterns while creating new components only where necessary. The approach prioritizes minimal core changes, type safety through enum extensions, and progressive enhancement from basic API integration to advanced features.

## Phase 0: Specification Corrections

### Overview
Fix all inaccuracies in the specifications before starting implementation to ensure clarity.

### Changes Required:

#### 1. Update specs/anthropic_support/07_file_editing_alternatives.md
**File**: `codex-rs/specs/anthropic_support/07_file_editing_alternatives.md`
**Changes**:
- Add "**CURRENT STATE:**" header before describing ReadFile, ListDir, GrepFiles
- Add "**PROPOSED:**" header before FileEditingStrategy enum
- Clearly mark WriteFile, EditFile, CreateFile, DeleteFile as "TO BE CREATED"
- Update examples to show FileEditingStrategy as a new field

#### 2. Update specs/anthropic_support/08_model_families.md
**File**: `codex-rs/specs/anthropic_support/08_model_families.md`
**Changes**:
- Mark `file_editing_strategy` field with `// NEW - to be added`
- Mark `max_output_tokens` as `Option<u32>` with `// NEW - to be added`
- Add migration notes for existing model family definitions

#### 3. Update specs/anthropic_support/10_credential_persistence.md
**File**: `codex-rs/specs/anthropic_support/10_credential_persistence.md`
**Changes**:
- Add `#[serde(default)]` to all new Anthropic fields
- Remove `.credential.json` references, use `auth.json` consistently
- Add explicit backward compatibility section

#### 4. Verify API Version
**Action**: Check if `anthropic-version: 2023-06-01` is still current
**Update**: specs/anthropic_support/03_provider_integration.md with latest version

### Success Criteria:

#### Automated Verification:
- [x] All spec files pass markdown linting
- [x] Code examples compile when extracted

#### Manual Verification:
- [ ] Specifications clearly distinguish current vs proposed
- [ ] No references to non-existent structures
- [ ] API version confirmed as current

---

## Phase 1: Core Provider Integration

### Overview
Add the Anthropic provider to the system with basic configuration and wire API support.

### Changes Required:

#### 1. Extend WireApi Enum
**File**: `codex-rs/core/src/model_provider_info.rs`
**Changes**: Add Anthropic variant to WireApi enum at line 31

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WireApi {
    Responses,
    #[default]
    Chat,
    Anthropic,  // NEW
}
```

#### 2. Update Pattern Matches
**File**: `codex-rs/core/src/model_provider_info.rs:159`
**Changes**: Add Anthropic case to get_full_url()

```rust
match self.wire_api {
    WireApi::Responses => "/responses",
    WireApi::Chat => "/chat/completions",
    WireApi::Anthropic => "/v1/messages",  // NEW
}
```

**File**: `codex-rs/core/src/client.rs:139`
**Changes**: Add Anthropic dispatch case

```rust
match self.provider.wire_api {
    WireApi::Responses => self.stream_responses(prompt, task_kind).await,
    WireApi::Chat => { /* existing */ }
    WireApi::Anthropic => {
        stream_anthropic_messages(
            prompt,
            &self.config.model_family,
            &self.client,
            &self.provider,
            &self.auth,
            &self.otel_event_manager,
        ).await
    }
}
```

#### 3. Register Built-in Provider
**File**: `codex-rs/core/src/model_provider_info.rs:255`
**Changes**: Add Anthropic to built_in_model_providers()

```rust
("anthropic", ModelProviderInfo {
    name: "Anthropic".to_string(),
    base_url: Some("https://api.anthropic.com".to_string()),
    env_key: Some("ANTHROPIC_API_KEY".to_string()),
    env_key_instructions: Some(
        "Get your API key from https://console.anthropic.com/settings/keys".to_string()
    ),
    wire_api: WireApi::Anthropic,
    http_headers: Some(HashMap::from([
        ("anthropic-version".to_string(), "2023-06-01".to_string()),
    ])),
    requires_openai_auth: false,
    request_max_retries: Some(4),
    stream_max_retries: Some(5),
    stream_idle_timeout_ms: Some(300_000),
    ..Default::default()
})
```

#### 4. Update Authentication Headers
**File**: `codex-rs/core/src/model_provider_info.rs:121`
**Changes**: Modify create_request_builder() to use x-api-key for Anthropic

```rust
if let Some(api_key) = &effective_auth.api_key {
    match self.wire_api {
        WireApi::Anthropic => {
            request_builder = request_builder.header("x-api-key", api_key);
        }
        _ => {
            request_builder = request_builder.bearer_auth(api_key);
        }
    }
}
```

### Success Criteria:

#### Automated Verification:
- [x] Compilation succeeds: `cd codex-rs && cargo build -p codex-core`
- [x] Tests pass: `cargo test -p codex-core`
- [x] Pattern match exhaustiveness check passes

#### Manual Verification:
- [x] Provider appears in available providers list
- [x] Configuration loads correctly from TOML
- [x] API key loads from environment variable

---

## Phase 2: Messages API Implementation

### Overview
Implement the core Anthropic Messages API with request conversion and streaming support.

### Changes Required:

#### 1. Create Anthropic Messages Module
**File**: `codex-rs/core/src/anthropic_messages.rs` (NEW)
**Changes**: Full implementation of Messages API client

```rust
use crate::client_common::{Prompt, ResponseEvent, ResponseStream};
use crate::error::{CodexErr, Result};
use crate::model_family::ModelFamily;
use crate::ModelProviderInfo;

pub(crate) async fn stream_anthropic_messages(
    prompt: &Prompt,
    model_family: &ModelFamily,
    client: &reqwest::Client,
    provider: &ModelProviderInfo,
    auth: &Option<CodexAuth>,
    otel_event_manager: &OtelEventManager,
) -> Result<ResponseStream> {
    // Implementation details...
}
```

#### 2. Implement Request Conversion
**File**: `codex-rs/core/src/anthropic_messages.rs`
**Function**: `build_anthropic_request()`

Key conversions:
- Extract system prompt to `system` field
- Convert messages with role alternation enforcement
- Add required `max_tokens` field (4096 default)
- Convert tools to `input_schema` format

#### 3. Implement SSE Parser
**File**: `codex-rs/core/src/anthropic_messages.rs`
**Function**: `process_anthropic_sse()`

Handle Anthropic's named events:
- `message_start`: Initialize state
- `content_block_start`: Track block type (text/tool_use)
- `content_block_delta`: Accumulate deltas
- `content_block_stop`: Finalize blocks
- `message_stop`: Complete stream

#### 4. Add Module to lib.rs
**File**: `codex-rs/core/src/lib.rs`
**Changes**: Add module declaration

```rust
mod anthropic_messages;
```

### Success Criteria:

#### Automated Verification:
- [x] Unit tests for request conversion pass
- [x] Unit tests for SSE parsing pass
- [x] Integration test with mock Anthropic server passes

#### Manual Verification:
- [ ] Basic prompt works with Claude model
- [ ] Streaming displays correctly in TUI
- [ ] Error messages are meaningful

---

## Phase 3: Tool System Adaptation

### Overview
Implement tool conversion and create the missing file editing tools for Claude.

### Changes Required:

#### 1. Add Tool Conversion Function
**File**: `codex-rs/core/src/tools/spec.rs`
**Changes**: Add at line 702 after chat completions converter

```rust
pub(crate) fn create_tools_json_for_anthropic_api(
    specs: &[ConfiguredToolSpec],
) -> Result<Vec<serde_json::Value>> {
    specs
        .iter()
        .filter_map(|spec| match &spec.spec {
            ToolSpec::Function(tool) => {
                let mut anthropic_tool = json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.parameters,
                });
                Some(Ok(anthropic_tool))
            }
            _ => None,  // Skip LocalShell, WebSearch, Freeform
        })
        .collect()
}
```

#### 2. Create WriteFile Tool
**File**: `codex-rs/core/src/tools/handlers/write_file.rs` (NEW)
**Changes**: Implement WriteFile handler

```rust
pub struct WriteFileHandler;

impl ToolHandler for WriteFileHandler {
    fn kind(&self) -> ToolKind { ToolKind::Function }

    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        // Parse arguments for file_path and content
        // Write to filesystem
        // Return success/failure
    }
}
```

#### 3. Create EditFile Tool
**File**: `codex-rs/core/src/tools/handlers/edit_file.rs` (NEW)
**Changes**: Implement EditFile handler with unique text matching

```rust
pub struct EditFileHandler;

impl ToolHandler for EditFileHandler {
    async fn handle(&self, invocation: ToolInvocation) -> Result<ToolOutput, FunctionCallError> {
        // Parse file_path, old_text, new_text
        // Verify old_text is unique in file
        // Perform replacement
        // Return result
    }
}
```

#### 4. Create DeleteFile Tool
**File**: `codex-rs/core/src/tools/handlers/delete_file.rs` (NEW)
**Changes**: Implement safe file deletion with confirmation

#### 5. Add FileEditingStrategy Enum
**File**: `codex-rs/core/src/model_family.rs`
**Changes**: Add new enum and field to ModelFamily

```rust
pub enum FileEditingStrategy {
    ApplyPatch,
    SeparateTools,
}

pub struct ModelFamily {
    // existing fields...
    pub file_editing_strategy: Option<FileEditingStrategy>,  // NEW
    pub max_output_tokens: Option<u32>,  // NEW
}
```

#### 6. Register New Tools
**File**: `codex-rs/core/src/tools/spec.rs:814`
**Changes**: Add new tools to build_specs()

```rust
if model_family.file_editing_strategy == Some(FileEditingStrategy::SeparateTools) {
    // Register WriteFile, EditFile, DeleteFile
}
```

### Success Criteria:

#### Automated Verification:
- [x] Tool conversion tests pass
- [x] WriteFile handler tests pass
- [x] EditFile handler tests pass
- [x] DeleteFile handler tests pass

#### Manual Verification:
- [ ] Claude can successfully call and use file editing tools
- [ ] Tool results are properly formatted
- [ ] Error handling works correctly

---

## Phase 3.5: TUI Display Integration

### Overview
Integrate file editing tools with TUI to display them using the same visual pattern as apply_patch, ensuring a consistent user experience. This phase converts tool calls into FileChange format and reuses all existing apply_patch rendering infrastructure.

### Design Approach
Instead of creating new display components, we'll convert write_file, edit_file, and delete_file tool calls into the existing `FileChange` enum format that apply_patch uses. This allows complete reuse of:
- `PatchHistoryCell` for rendering
- `create_diff_summary()` for formatting
- `ApprovalOverlay` for user confirmation
- All diff styling and coloring logic

The result: File editing tools will be visually identical to apply_patch operations.

### Changes Required:

#### 1. Add New Protocol Events
**File**: `codex-rs/protocol/src/protocol.rs`
**Changes**: Add events for file editing operations (similar to PatchApply events)

```rust
// After PatchApplyEndEvent (around line 1230)

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename = "file_edit_approval_request")]
pub struct FileEditApprovalRequestEvent {
    pub call_id: String,
    pub changes: HashMap<PathBuf, FileChange>,  // Reuse existing FileChange!
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename = "file_edit_begin")]
pub struct FileEditBeginEvent {
    pub call_id: String,
    pub auto_approved: bool,
    pub changes: HashMap<PathBuf, FileChange>,  // Reuse existing FileChange!
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename = "file_edit_end")]
pub struct FileEditEndEvent {
    pub call_id: String,
    pub success: bool,
    pub stderr: String,
}
```

Add to EventMsg enum (around line 1395):
```rust
#[serde(rename = "file_edit_approval_request")]
FileEditApprovalRequest(FileEditApprovalRequestEvent),
#[serde(rename = "file_edit_begin")]
FileEditBegin(FileEditBeginEvent),
#[serde(rename = "file_edit_end")]
FileEditEnd(FileEditEndEvent),
```

#### 2. Convert Tool Calls to FileChange Format
**File**: `codex-rs/core/src/tools/handlers/write_file.rs`
**Changes**: Add conversion function

```rust
use std::collections::HashMap;
use std::path::PathBuf;
use codex_protocol::FileChange;

impl WriteFileHandler {
    /// Convert write_file call to FileChange for TUI display
    fn to_file_change(file_path: &str, content: &str) -> (PathBuf, FileChange) {
        (
            PathBuf::from(file_path),
            FileChange::Add {
                content: content.to_string(),
            }
        )
    }
}
```

**File**: `codex-rs/core/src/tools/handlers/edit_file.rs`
**Changes**: Add unified diff generation

```rust
use diffy::{create_patch, PatchFormatter};

impl EditFileHandler {
    /// Convert edit_file call to FileChange with unified diff
    fn to_file_change(
        file_path: &str,
        old_content: &str,
        new_content: &str
    ) -> (PathBuf, FileChange) {
        let patch = create_patch(old_content, new_content);
        let unified_diff = format!("{}", PatchFormatter::new().with_color());

        (
            PathBuf::from(file_path),
            FileChange::Update {
                unified_diff,
                move_path: None,
            }
        )
    }
}
```

**File**: `codex-rs/core/src/tools/handlers/delete_file.rs`
**Changes**: Read file before deleting for diff display

```rust
impl DeleteFileHandler {
    /// Convert delete_file call to FileChange
    async fn to_file_change(file_path: &str) -> Result<(PathBuf, FileChange), FunctionCallError> {
        let content = tokio::fs::read_to_string(file_path)
            .await
            .map_err(|e| FunctionCallError::RespondToModel(format!("Failed to read file: {e}")))?;

        Ok((
            PathBuf::from(file_path),
            FileChange::Delete { content }
        ))
    }
}
```

#### 3. Batch and Emit Events
**File**: `codex-rs/core/src/tools/handlers/mod.rs` (or new file for batching logic)
**Changes**: Add batching logic to group sequential file operations

```rust
pub struct FileEditBatcher {
    pending_changes: HashMap<PathBuf, FileChange>,
    timer: Option<Instant>,
}

impl FileEditBatcher {
    const BATCH_WINDOW_MS: u64 = 100;  // Wait 100ms to batch operations

    pub fn add_change(&mut self, path: PathBuf, change: FileChange) {
        self.pending_changes.insert(path, change);
        if self.timer.is_none() {
            self.timer = Some(Instant::now());
        }
    }

    pub fn should_flush(&self) -> bool {
        self.timer
            .map(|t| t.elapsed().as_millis() > Self::BATCH_WINDOW_MS as u128)
            .unwrap_or(false)
    }

    pub fn flush(&mut self) -> HashMap<PathBuf, FileChange> {
        self.timer = None;
        std::mem::take(&mut self.pending_changes)
    }
}
```

#### 4. Integrate with Tool Execution Flow
**File**: Location where tool calls are processed (likely in `codex-rs/core/src/`)
**Changes**: Intercept file editing tool calls before execution

```rust
// When a file editing tool is called:

1. Convert to FileChange
2. Add to batcher
3. If approval needed:
   - Emit FileEditApprovalRequestEvent
   - Wait for user approval
4. Emit FileEditBeginEvent
5. Execute actual file operations
6. Emit FileEditEndEvent
```

#### 5. Add TUI Event Handlers
**File**: `codex-rs/tui/src/chatwidget.rs`
**Changes**: Add handlers that reuse apply_patch logic

After `handle_patch_apply_end_now` (around line 780):
```rust
fn on_file_edit_approval_request(&mut self, event: FileEditApprovalRequestEvent) {
    // Reuse exact same approval modal as apply_patch!
    self.show_approval_request(ApprovalRequest::FileEdit {
        id: event.call_id,
        reason: event.reason,
        cwd: self.config.cwd.clone(),
        changes: event.changes,
    });
}

fn on_file_edit_begin(&mut self, event: FileEditBeginEvent) {
    // Reuse exact same history cell as apply_patch!
    self.add_to_history(history_cell::new_patch_event(
        event.changes,
        &self.config.cwd,
    ));
}

fn on_file_edit_end(&mut self, event: FileEditEndEvent) {
    // Reuse exact same failure display as apply_patch!
    if !event.success {
        self.add_to_history(history_cell::new_patch_apply_failure(event.stderr));
    }
}
```

Add to `dispatch_event_msg` (around line 1450):
```rust
EventMsg::FileEditApprovalRequest(event) => {
    self.on_file_edit_approval_request(event);
}
EventMsg::FileEditBegin(event) => {
    self.on_file_edit_begin(event);
}
EventMsg::FileEditEnd(event) => {
    self.on_file_edit_end(event);
}
```

#### 6. Update Approval Overlay
**File**: `codex-rs/tui/src/bottom_pane/approval_overlay.rs`
**Changes**: Add FileEdit variant (around line 50)

```rust
pub enum ApprovalVariant {
    Shell { id: String },
    ApplyPatch { id: String },
    FileEdit { id: String },  // NEW - handles the same as ApplyPatch
}
```

Add to ApprovalRequest enum (after ApplyPatch variant):
```rust
ApprovalRequest::FileEdit {
    id,
    reason,
    cwd,
    changes,
} => {
    // Identical to ApplyPatch handling
    let mut header: Vec<Box<dyn Renderable>> = Vec::new();
    if let Some(reason) = reason && !reason.is_empty() {
        header.push(Box::new(
            Paragraph::new(Line::from_iter(["Reason: ".into(), reason.italic()]))
                .wrap(Wrap { trim: false }),
        ));
        header.push(Box::new(Line::from("")));
    }
    header.push(DiffSummary::new(changes, cwd).into());
    Self {
        variant: ApprovalVariant::FileEdit { id },
        header: Box::new(ColumnRenderable::with(header)),
    }
}
```

### Success Criteria:

#### Automated Verification:
- [x] Protocol events compile and serialize correctly: `cargo build -p codex-protocol`
- [x] TUI compiles with new event handlers: `cargo build -p codex-tui`
- [x] Tool handler tests still pass: `cargo test -p codex-core`
- [x] No regressions in existing apply_patch display: `cargo test -p codex-tui`

#### Manual Verification:
- [ ] write_file displays as "• Added filename.txt (+N -0)" with green additions
- [ ] edit_file displays as "• Edited filename.txt (+N -M)" with diff context
- [ ] delete_file displays as "• Deleted filename.txt (+0 -N)" with red deletions
- [ ] Multiple file operations batch and display as "• Edited 3 files (+X -Y)"
- [ ] Approval modal shows identical to apply_patch with file changes preview
- [ ] Failed operations show "✘ Failed to write/edit/delete file" in magenta
- [ ] Display is visually indistinguishable from apply_patch operations
- [ ] No "Called write_file" or tool invocation details shown

**Implementation Note**: The key insight is that file editing tools are displayed as file changes, not as tool calls. Users see "• Edited config.json" not "• Called edit_file". This maintains consistency with apply_patch and provides a better UX.

---

## Phase 3.6: Core Event Integration

### Overview
Connect file editing tools to the event emission system so they display in the TUI. This phase makes the file editing tools emit `FileEditBeginEvent` and `FileEditEndEvent` during execution, enabling the TUI infrastructure from Phase 3.5 to display them.

### Problem Statement
Currently, file editing tools (write_file, edit_file, delete_file) execute successfully but don't emit any events. The TUI infrastructure exists (Phase 3.5) but never receives events to display. Tool handlers cannot emit events directly - only the orchestration layer (`Session` in `codex.rs`) can emit events.

### Design Approach
Since tool handlers cannot emit events directly, we need to intercept file editing tool calls at the orchestration layer and emit events before and after execution. This follows a similar pattern to how `apply_patch` emits `PatchApplyBeginEvent` and `PatchApplyEndEvent` through the exec system.

### Changes Required:

#### 1. Add File Edit Detection in Tool Processing
**File**: `codex-rs/core/src/tools/router.rs`
**Changes**: Add method to detect file editing tools

```rust
impl ToolRouter {
    /// Check if a tool call is a file editing operation
    pub fn is_file_edit_tool(tool_name: &str) -> bool {
        matches!(tool_name, "write_file" | "edit_file" | "delete_file")
    }
}
```

#### 2. Modify Tool Call Runtime to Track File Edits
**File**: `codex-rs/core/src/tools/parallel.rs`
**Changes**: Enhance `ToolCallRuntime` to detect and convert file editing tools

```rust
impl ToolCallRuntime {
    pub async fn handle_tool_call(
        &self,
        call: ToolCall,
    ) -> Result<ProcessedResponseItem, FunctionCallError> {
        let tool_name = call.tool_name();

        // Check if this is a file editing tool
        if ToolRouter::is_file_edit_tool(&tool_name) {
            // Emit FileEditBeginEvent before execution
            self.emit_file_edit_begin(&call).await;
        }

        // Execute tool as normal
        let result = /* existing execution logic */;

        // Emit FileEditEndEvent after execution
        if ToolRouter::is_file_edit_tool(&tool_name) {
            self.emit_file_edit_end(&call, &result).await;
        }

        result
    }
}
```

#### 3. Add Event Emission Methods to Session
**File**: `codex-rs/core/src/codex.rs`
**Changes**: Add methods to emit file edit events (around line 1010)

```rust
impl Session {
    /// Emit FileEditBeginEvent when a file editing tool starts
    async fn on_file_edit_begin(
        &self,
        call_id: String,
        tool_name: String,
        arguments: String,
    ) {
        // Parse arguments to extract file path and convert to FileChange
        let changes = match tool_name.as_str() {
            "write_file" => {
                // Parse file_path and content from arguments
                // Create FileChange::Add
            }
            "edit_file" => {
                // Parse file_path, old_text, new_text
                // Read current file content
                // Create FileChange::Update with diff
            }
            "delete_file" => {
                // Parse file_path
                // Read file content before deletion
                // Create FileChange::Delete
            }
            _ => return,
        };

        let event = Event::new(EventMsg::FileEditBegin(FileEditBeginEvent {
            call_id,
            auto_approved: true,  // File edits are currently auto-approved
            changes,
        }));

        self.send_event(event).await;
    }

    /// Emit FileEditEndEvent when a file editing tool completes
    async fn on_file_edit_end(
        &self,
        call_id: String,
        success: bool,
        error_message: Option<String>,
    ) {
        let event = Event::new(EventMsg::FileEditEnd(FileEditEndEvent {
            call_id,
            success,
            stderr: error_message.unwrap_or_default(),
        }));

        self.send_event(event).await;
    }
}
```

#### 4. Pass Session Reference to Tool Runtime
**File**: `codex-rs/core/src/codex.rs:2177`
**Changes**: Modify `ToolCallRuntime` creation to include event emission capability

```rust
// Current code creates ToolCallRuntime without event access
// Need to either:
// Option A: Pass session reference or event channel to ToolCallRuntime
// Option B: Add callbacks for event emission
// Option C: Return metadata from tool execution for Session to emit events
```

#### 5. Create FileChange Conversion Utilities
**File**: `codex-rs/core/src/tools/file_change_converter.rs` (NEW)
**Changes**: Add utilities to convert tool arguments to FileChange

```rust
use codex_protocol::FileChange;
use std::collections::HashMap;
use std::path::PathBuf;

/// Convert write_file arguments to FileChange
pub fn write_file_to_file_change(
    file_path: String,
    content: String,
) -> HashMap<PathBuf, FileChange> {
    let mut changes = HashMap::new();
    changes.insert(
        PathBuf::from(file_path),
        FileChange::Add { content },
    );
    changes
}

/// Convert edit_file to FileChange with diff
pub fn edit_file_to_file_change(
    file_path: String,
    old_content: &str,
    new_content: &str,
) -> HashMap<PathBuf, FileChange> {
    use diffy::{create_patch, PatchFormatter};

    let patch = create_patch(old_content, new_content);
    let unified_diff = format!("{}", PatchFormatter::new().with_color(false));

    let mut changes = HashMap::new();
    changes.insert(
        PathBuf::from(file_path),
        FileChange::Update {
            unified_diff,
            move_path: None,
        },
    );
    changes
}

/// Convert delete_file to FileChange
pub fn delete_file_to_file_change(
    file_path: String,
    content: String,
) -> HashMap<PathBuf, FileChange> {
    let mut changes = HashMap::new();
    changes.insert(
        PathBuf::from(file_path),
        FileChange::Delete { content },
    );
    changes
}
```

### Implementation Challenges

1. **Tool Runtime Architecture**: The current `ToolCallRuntime` doesn't have access to the event channel. Need to refactor to either:
   - Pass event channel to tool runtime
   - Use callbacks for event emission
   - Return metadata for Session to emit events post-execution

2. **File Content Access**: For edit_file and delete_file, we need to read the current file content to generate proper diffs. This requires async file I/O during event emission.

3. **Error Handling**: Need to handle cases where file reading fails during FileChange conversion.

### Success Criteria:

#### Automated Verification:
- [x] File editing tools still function correctly: `cargo test -p codex-core`
- [x] Events are emitted during tool execution
- [x] FileChange conversion works for all three tools

#### Manual Verification:
- [ ] Running `write_file` shows "• Added filename.txt" in TUI
- [ ] Running `edit_file` shows "• Edited filename.txt" with diff
- [ ] Running `delete_file` shows "• Deleted filename.txt"
- [ ] Events appear immediately when tools execute
- [ ] No regression in other tool displays

---

## Phase 4: Model Family Configuration

### Overview
Add Claude model families with appropriate configuration.

### Changes Required:

#### 1. Add Claude Model Families
**File**: `codex-rs/core/src/model_family.rs`
**Function**: `find_family_for_model()`

```rust
if model == "claude-3-5-sonnet-20241022" || model.starts_with("claude-3-5-sonnet") {
    return Some(ModelFamily {
        slug: model.to_string(),
        family: "claude-3.5".to_string(),
        supports_parallel_tool_calls: true,
        apply_patch_tool_type: None,
        file_editing_strategy: Some(FileEditingStrategy::SeparateTools),
        max_output_tokens: Some(8192),
        // ... other fields
    });
}
```

Add similar blocks for:
- claude-3-opus
- claude-3-haiku
- claude-3-5-haiku

### Success Criteria:

#### Automated Verification:
- [x] Model family lookup tests pass
- [x] Serialization/deserialization works

#### Manual Verification:
- [ ] Models are recognized when specified
- [ ] Correct tools are available per model
- [ ] max_tokens is properly set

---

## Phase 5: Authentication Enhancement

### Overview
Add OAuth support for Claude Code integration (can be implemented after basic API key support is working).

### Changes Required:

#### 1. Extend AuthMode Enum
**File**: `codex-rs/app-server-protocol/src/protocol.rs:36`
**Changes**: Add ClaudeOAuth variant

```rust
pub enum AuthMode {
    ApiKey,
    ChatGPT,
    ClaudeOAuth,  // NEW
}
```

#### 2. Extend AuthDotJson Structure
**File**: `codex-rs/core/src/auth.rs:379`
**Changes**: Add Anthropic OAuth fields with backward compatibility

```rust
pub struct AuthDotJson {
    // existing fields...

    #[serde(default, rename = "ANTHROPIC_API_KEY", skip_serializing_if = "Option::is_none")]
    pub anthropic_api_key: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_tokens: Option<AnthropicTokenData>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_last_refresh: Option<DateTime<Utc>>,
}
```

#### 3. Implement OAuth Flow
**File**: `codex-rs/core/src/auth_anthropic.rs` (NEW)
**Changes**: Implement PKCE OAuth flow for Claude Code

### Success Criteria:

#### Automated Verification:
- [ ] Auth struct serialization maintains backward compatibility
- [ ] OAuth flow tests pass with mock server

#### Manual Verification:
- [ ] OAuth login flow completes successfully
- [ ] Tokens are stored and refreshed correctly
- [ ] Existing auth.json files still work

---

## Phase 6: Testing and Polish

### Overview
Comprehensive testing, error handling improvements, and documentation.

### Changes Required:

#### 1. Integration Tests
**File**: `codex-rs/core/tests/anthropic_integration.rs` (NEW)
- Mock server tests for all endpoints
- Streaming event parsing tests
- Tool calling round-trip tests

#### 2. Error Message Improvements
**File**: `codex-rs/core/src/anthropic_messages.rs`
- Parse Anthropic error responses
- Provide helpful user-facing messages
- Handle rate limits appropriately

#### 3. Documentation
**File**: `codex-rs/docs/anthropic.md` (NEW)
- Configuration guide
- Model selection guide
- Troubleshooting section

#### 4. Migration Guide
**File**: `codex-rs/docs/migration_to_anthropic.md` (NEW)
- For users switching from OpenAI
- Performance comparisons
- Feature differences

### Success Criteria:

#### Automated Verification:
- [ ] All integration tests pass
- [ ] No regressions in existing tests
- [ ] Documentation builds correctly

#### Manual Verification:
- [ ] End-to-end workflows function correctly
- [ ] Error messages are helpful
- [ ] Documentation is clear and complete

---

## Testing Strategy

### Unit Tests:
- Request conversion logic
- SSE event parsing
- Tool schema conversion
- File operation handlers

### Integration Tests:
- Full request/response cycles with mock server
- Multi-turn conversations with tool use
- OAuth flow with mock authorization server
- File editing operations in sandboxed environment

### Manual Testing Steps:
1. Configure Anthropic provider with API key
2. Test basic prompt: `codex --model claude-3-5-sonnet-20241022 --provider anthropic "Hello"`
3. Test tool calling with file operations
4. Test streaming with long responses
5. Test error handling with invalid API key
6. Test OAuth flow (Phase 5)

## Performance Considerations

- SSE parsing should maintain similar latency to OpenAI streams
- File operations should be atomic and efficient
- Token counting for max_tokens should be conservative
- Rate limit handling should use exponential backoff

## Migration Notes

For existing users:
1. No changes required if continuing to use OpenAI
2. To try Anthropic: set `ANTHROPIC_API_KEY` and add `--provider anthropic`
3. Existing auth.json files remain compatible
4. Configuration can specify default provider

## References

- Original specifications: `codex-rs/specs/anthropic_support/`
- Feasibility research: `thoughts/shared/research/2025-10-19-anthropic-support-feasibility.md`
- Current provider implementation: `codex-rs/core/src/model_provider_info.rs`
- Current streaming implementation: `codex-rs/core/src/chat_completions.rs`

## Timeline Estimate

Based on complexity analysis:
- Phase 0: 1 day (spec corrections)
- Phase 1: 2 days (provider integration)
- Phase 2: 4-5 days (Messages API)
- Phase 3: 5-7 days (tool system and file tools)
- Phase 3.5: 2-3 days (TUI display infrastructure)
- Phase 3.6: 3-4 days (core event integration)
- Phase 4: 1 day (model families)
- Phase 5: 3-4 days (OAuth - can be deferred)
- Phase 6: 3-4 days (testing and polish)

**Total: 24-35 days** (4-5 weeks for core functionality with full TUI integration, +1 week for OAuth)

## Risk Mitigation

### Technical Risks:
1. **File editing tool effectiveness**: Prototype early, test with actual Claude API
2. **Message alternation enforcement**: Implement robust message merging logic
3. **SSE parsing differences**: Use existing eventsource_stream capabilities

### Process Risks:
1. **Specification drift**: Fix specs first before implementation
2. **Backward compatibility**: Extensive testing of existing flows
3. **Performance degradation**: Benchmark against OpenAI baseline