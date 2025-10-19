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
- [ ] Provider appears in available providers list
- [ ] Configuration loads correctly from TOML
- [ ] API key loads from environment variable

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
- [ ] Unit tests for request conversion pass
- [ ] Unit tests for SSE parsing pass
- [ ] Integration test with mock Anthropic server passes

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
- [ ] Tool conversion tests pass
- [ ] WriteFile handler tests pass
- [ ] EditFile handler tests pass
- [ ] DeleteFile handler tests pass

#### Manual Verification:
- [ ] Claude can successfully call and use file editing tools
- [ ] Tool results are properly formatted
- [ ] Error handling works correctly

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
- [ ] Model family lookup tests pass
- [ ] Serialization/deserialization works

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
- Phase 4: 1 day (model families)
- Phase 5: 3-4 days (OAuth - can be deferred)
- Phase 6: 3-4 days (testing and polish)

**Total: 19-27 days** (3-4 weeks for core functionality, +1 week for OAuth)

## Risk Mitigation

### Technical Risks:
1. **File editing tool effectiveness**: Prototype early, test with actual Claude API
2. **Message alternation enforcement**: Implement robust message merging logic
3. **SSE parsing differences**: Use existing eventsource_stream capabilities

### Process Risks:
1. **Specification drift**: Fix specs first before implementation
2. **Backward compatibility**: Extensive testing of existing flows
3. **Performance degradation**: Benchmark against OpenAI baseline