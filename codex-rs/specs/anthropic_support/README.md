# Anthropic Support Specification

## Overview

This directory contains comprehensive specifications for adding Anthropic's Claude API support to the Codex-RS codebase. The specifications cover all aspects of integration including API differences, authentication, tool calling, file editing, and implementation planning.

## Document Structure

### 1. [Overview](01_overview.md)
**Purpose**: High-level introduction to the Anthropic integration project

**Key Topics**:
- Motivation for adding Anthropic support
- Scope and goals
- Architecture overview
- Success criteria
- Timeline estimates

**Read this first** to understand the project goals and approach.

---

### 2. [API Comparison](02_api_comparison.md)
**Purpose**: Detailed comparison between Anthropic Messages API and OpenAI APIs

**Key Topics**:
- Request/response format differences
- System instructions handling
- Message structure
- Tool definitions (parameters vs input_schema)
- Streaming formats (SSE events)
- Tool result submission
- Conversation alternation rules

**Critical for understanding** how to translate between APIs.

---

### 3. [Provider Integration](03_provider_integration.md)
**Purpose**: How to integrate Anthropic as a ModelProvider

**Key Topics**:
- WireApi enum extension
- Anthropic provider configuration
- Request builder changes
- Client dispatch logic
- Error handling
- Rate limit tracking

**Implementation guide** for core provider setup.

---

### 4. [Authentication](04_authentication.md)
**Purpose**: Authentication mechanisms for Anthropic API

**Key Topics**:
- Simple API key authentication
- OAuth 2.0 + PKCE flow for Claude Code
- Authorization flow diagram
- Token refresh logic
- Security considerations

**Reference for** setting up auth in your environment.

---

### 5. [Tool Calling](05_tool_calling.md)
**Purpose**: Adapting Codex's tool system for Anthropic

**Key Topics**:
- Tool format conversion (parameters → input_schema)
- Tool use response parsing
- Tool result formatting
- Handling multiple tool calls
- Unsupported tool types

**Essential for** implementing function calling.

---

### 6. [Streaming Events](06_streaming_events.md)
**Purpose**: Parsing Anthropic SSE and mapping to ResponseEvent

**Key Topics**:
- Anthropic SSE event structure
- Event lifecycle (message_start → content_block → message_stop)
- State machine implementation
- Event mapping table
- Comparison with OpenAI streaming

**Detailed guide** for streaming implementation.

---

### 7. [File Editing Alternatives](07_file_editing_alternatives.md)
**Purpose**: Alternative file editing strategies for Claude models

**Key Topics**:
- Problems with ApplyPatch for Claude
- Separate Read/Write/Edit tools (recommended)
- Line-based editing
- Simplified patch format
- Implementation recommendations

**Critical for** making Claude effective at file editing.

---

### 8. [Model Families](08_model_families.md)
**Purpose**: Claude model family definitions

**Key Topics**:
- Claude 3.5 Sonnet (recommended)
- Claude 3 Opus (highest quality)
- Claude 3.5/3 Haiku (fast/cheap)
- Model capabilities and context windows
- Pricing considerations
- Default model selection

**Reference for** configuring Claude models.

---

### 9. [Implementation Roadmap](09_implementation_roadmap.md)
**Purpose**: Step-by-step implementation plan

**Key Topics**:
- Phase 1: Core API (2-3 weeks)
- Phase 2: Tool System (2 weeks)
- Phase 3: Auth & Polish (2 weeks)
- Phase 4: Testing (1 week)
- Task dependencies
- Risk mitigation
- Success metrics

**Project plan** with timeline and milestones.

---

### 10. [Credential Persistence](10_credential_persistence.md)
**Purpose**: Managing and persisting OAuth credentials across sessions

**Key Topics**:
- Credential file format (`.credential.json`)
- File location strategy (project-local vs global)
- Token refresh mechanism
- Security considerations (permissions, encryption)
- CLI commands for credential management
- Integration with auth system

**Essential for** avoiding re-authentication on every session.

---

## Quick Start

### For Implementers

1. Read [01_overview.md](01_overview.md) - Understand the goals
2. Read [02_api_comparison.md](02_api_comparison.md) - Learn the API differences
3. Follow [09_implementation_roadmap.md](09_implementation_roadmap.md) - Execute the plan

### For Reviewers

1. Start with [01_overview.md](01_overview.md) - Context
2. Review [09_implementation_roadmap.md](09_implementation_roadmap.md) - Timeline
3. Deep-dive into specific areas as needed

### For Users (Future)

Once implemented:
1. See [04_authentication.md](04_authentication.md) - Set up auth
2. See [08_model_families.md](08_model_families.md) - Choose a model
3. See User Documentation (to be created)

## Key Decisions

### Decision 1: WireApi Extension
**Status**: Recommended
**Details**: Add `WireApi::Anthropic` variant instead of reusing `WireApi::Chat`
**Rationale**: Type safety, clarity, future-proofing
**See**: [03_provider_integration.md](03_provider_integration.md#option-1-extend-wireapi-enum-recommended)

### Decision 2: File Editing Strategy
**Status**: Recommended
**Details**: Use separate Read/Write/Edit tools for Claude models
**Rationale**: Claude performs better with structured JSON tools
**See**: [07_file_editing_alternatives.md](07_file_editing_alternatives.md#option-1-separate-readwriteedit-tools-recommended)

### Decision 3: Authentication Priority
**Status**: Recommended
**Details**: Implement API key auth first, OAuth later
**Rationale**: API key is simpler, gets us to MVP faster
**See**: [04_authentication.md](04_authentication.md#authentication-methods)

### Decision 4: Default Model
**Status**: Recommended
**Details**: Default to `claude-3-5-sonnet-20241022`
**Rationale**: Best balance of capability, speed, and cost
**See**: [08_model_families.md](08_model_families.md#default-model-selection)

### Decision 5: Credential Persistence
**Status**: Recommended
**Details**: Use `.credential.json` format with hybrid location strategy
**Rationale**: Matches existing format, avoids re-authentication
**See**: [10_credential_persistence.md](10_credential_persistence.md#file-location-strategy)

## Key Differences from OpenAI

| Aspect | OpenAI | Anthropic |
|--------|--------|-----------|
| **Auth Header** | `Authorization: Bearer` | `x-api-key:` |
| **API Version** | Not required | `anthropic-version` header required |
| **System Prompt** | Message or instructions field | Separate `system` field |
| **Tool Schema** | `parameters` | `input_schema` |
| **Tool Results** | Separate message (role=tool) | User message with tool_result content |
| **Streaming** | Data-only SSE events | Named SSE events |
| **max_tokens** | Optional | **Required** |
| **Message Order** | Flexible | **Strict alternation** |

## Implementation Checklist

### Phase 1: Core API
- [ ] Add WireApi::Anthropic variant
- [ ] Register Anthropic provider
- [ ] Implement API key authentication
- [ ] Create anthropic_messages.rs module
- [ ] Implement message conversion
- [ ] Implement streaming parser
- [ ] Add Claude model families
- [ ] Test basic prompts

### Phase 2: Tool System
- [ ] Implement tool converter
- [ ] Parse tool_use content blocks
- [ ] Format tool results
- [ ] Create Read/Write/Edit tools
- [ ] Add FileEditingStrategy
- [ ] Test multi-turn tool use

### Phase 3: Auth & Polish
- [ ] Implement PKCE OAuth
- [ ] Add OAuth callback server
- [ ] Add login command
- [ ] Implement credential persistence
- [ ] Add token refresh on load
- [ ] Improve error messages
- [ ] Add retry logic
- [ ] Track rate limits
- [ ] Write documentation

### Phase 4: Testing
- [ ] Unit tests
- [ ] Integration tests
- [ ] Performance testing
- [ ] User acceptance testing
- [ ] Bug fixes
- [ ] Release preparation

## Files to Create/Modify

### New Files
- `codex-rs/core/src/anthropic_messages.rs`
- `codex-rs/core/src/anthropic_tools.rs`
- `codex-rs/core/src/auth_claude_oauth.rs`
- `codex-rs/core/src/credential_manager.rs` *(NEW)*
- `codex-rs/core/src/tools/handlers/file_editing.rs`
- `docs/anthropic.md`

### Modified Files
- `codex-rs/core/src/model_provider_info.rs`
- `codex-rs/core/src/model_family.rs`
- `codex-rs/core/src/client.rs`
- `codex-rs/core/src/auth.rs`
- `codex-rs/core/src/tools/spec.rs`
- `codex-rs/cli/src/main.rs`

## Related Documentation

### External Resources
- [Anthropic API Documentation](https://docs.claude.com/en/api/messages)
- [Anthropic Tool Use Guide](https://docs.claude.com/en/docs/build-with-claude/tool-use)
- [OAuth 2.0 PKCE Specification](https://tools.ietf.org/html/rfc7636)

### Internal Resources
- [Current Protocol Documentation](../../codex-rs/docs/protocol_v1.md)
- [Current Model Provider Code](../../codex-rs/core/src/model_provider_info.rs)
- [Current Chat Completions Implementation](../../codex-rs/core/src/chat_completions.rs)

## FAQ

### Q: Why not use Responses API for both providers?
**A**: Anthropic doesn't have a Responses API equivalent. Messages API is their primary interface.

### Q: Can we support both API key and OAuth?
**A**: Yes, similar to OpenAI. API key for simplicity, OAuth for Claude Pro/Max users.

### Q: Will this break existing OpenAI integration?
**A**: No, changes are additive. OpenAI paths remain unchanged.

### Q: What about Amazon Bedrock or GCP Vertex AI?
**A**: Out of scope for initial release. Can be added later as separate providers.

### Q: How do we handle Anthropic-specific features like prompt caching?
**A**: Phase 1 focuses on core functionality. Advanced features like prompt caching can be added in future phases.

## Contributing

When adding to these specs:
1. Keep each document focused on a single topic
2. Provide code examples where helpful
3. Link to related documents
4. Update this README with new documents

## Status

**Last Updated**: 2025-01-19
**Status**: Draft - Ready for Review
**Next Steps**: Team review, approval, begin Phase 1 implementation

## Contact

For questions about these specifications, contact the Codex development team or open an issue in the repository.

---

**License**: Same as parent project
**Copyright**: Codex-RS contributors
