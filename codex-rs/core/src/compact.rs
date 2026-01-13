use std::sync::Arc;

use crate::ModelProviderInfo;
use crate::Prompt;
use crate::client_common::ResponseEvent;
use crate::codex::Session;
use crate::codex::TurnContext;
use crate::codex::get_last_assistant_message_from_turn;
use crate::error::CodexErr;
use crate::error::Result as CodexResult;
use crate::features::Feature;
use crate::protocol::CompactedItem;
use crate::protocol::ContextCompactedEvent;
use crate::protocol::EventMsg;
use crate::protocol::RestoredFileInfo;
use crate::protocol::TaskStartedEvent;
use crate::protocol::TurnContextItem;
use crate::protocol::WarningEvent;
use crate::truncate::TruncationPolicy;
use crate::truncate::approx_token_count;
use crate::truncate::truncate_text;
use crate::util::backoff;
use codex_protocol::items::TurnItem;
use codex_protocol::models::AttachmentData;
use codex_protocol::models::CompactRestoredFile;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::user_input::UserInput;
use futures::prelude::*;
use tracing::error;

use crate::read_file_state::ReadFileState;

pub const SUMMARIZATION_PROMPT: &str = include_str!("../templates/compact/prompt.md");
pub const SUMMARY_PREFIX: &str = include_str!("../templates/compact/summary_prefix.md");
const COMPACT_USER_MESSAGE_MAX_TOKENS: usize = 20_000;

/// Environment variable to override auto compact threshold percentage.
const CODEX_AUTOCOMPACT_PCT_OVERRIDE: &str = "CODEX_AUTOCOMPACT_PCT_OVERRIDE";

/// Default auto compact threshold as percentage of context window.
/// Set to 60% to trigger compaction earlier, preserving more context headroom.
const DEFAULT_AUTO_COMPACT_THRESHOLD_PCT: u8 = 60;

/// Buffer tokens to reserve for compaction overhead.
const AUTO_COMPACT_BUFFER_TOKENS: usize = 5000;

/// Maximum number of recent files to restore after compaction.
const MAX_RECENT_FILES_TO_RESTORE: usize = 5;

/// Maximum tokens per restored file. Files exceeding this become ReferenceOnly.
const MAX_TOKENS_PER_RESTORED_FILE: usize = 5000;

/// Maximum total tokens for all restored files combined.
const MAX_TOTAL_RESTORE_TOKENS: usize = 50000;

/// Get the effective auto compact threshold in tokens.
///
/// Priority order:
/// 1. `CODEX_AUTOCOMPACT_PCT_OVERRIDE` env var (highest priority)
/// 2. `model_auto_compact_token_limit` config (explicit absolute token limit)
/// 3. `auto_compact_threshold_pct` config (percentage-based)
/// 4. Default 60% of context window (lowest priority)
pub fn get_auto_compact_threshold(
    context_window: usize,
    model_auto_compact_token_limit: Option<i64>,
    auto_compact_threshold_pct: Option<u8>,
) -> usize {
    // 1. Check environment variable first (percentage-based override)
    if let Ok(env_val) = std::env::var(CODEX_AUTOCOMPACT_PCT_OVERRIDE) {
        if let Ok(pct) = env_val.parse::<f64>() {
            if pct > 0.0 && pct <= 100.0 {
                let from_pct = (context_window as f64 * (pct / 100.0)) as usize;
                return from_pct.saturating_sub(AUTO_COMPACT_BUFFER_TOKENS);
            }
        }
    }

    // 2. Use explicit absolute token limit if set (existing config)
    if let Some(limit) = model_auto_compact_token_limit {
        if limit > 0 {
            return limit as usize;
        }
    }

    // 3. Use percentage-based threshold from config
    let pct = auto_compact_threshold_pct.unwrap_or(DEFAULT_AUTO_COMPACT_THRESHOLD_PCT);
    let threshold = (context_window as f64 * (pct as f64 / 100.0)) as usize;
    threshold.saturating_sub(AUTO_COMPACT_BUFFER_TOKENS)
}

/// Check if auto compact should be triggered.
pub fn should_auto_compact(
    current_tokens: usize,
    context_window: usize,
    model_auto_compact_token_limit: Option<i64>,
    auto_compact_threshold_pct: Option<u8>,
    auto_compact_enabled: bool,
) -> bool {
    if !auto_compact_enabled {
        return false;
    }

    let threshold = get_auto_compact_threshold(
        context_window,
        model_auto_compact_token_limit,
        auto_compact_threshold_pct,
    );
    current_tokens >= threshold
}

pub(crate) fn should_use_remote_compact_task(
    session: &Session,
    provider: &ModelProviderInfo,
) -> bool {
    provider.is_openai() && session.enabled(Feature::RemoteCompaction)
}

pub(crate) async fn run_inline_auto_compact_task(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
) {
    let prompt = turn_context.compact_prompt().to_string();
    let input = vec![UserInput::Text { text: prompt }];

    run_compact_task_inner(sess, turn_context, input).await;
}

pub(crate) async fn run_compact_task(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    input: Vec<UserInput>,
) {
    let start_event = EventMsg::TaskStarted(TaskStartedEvent {
        model_context_window: turn_context.client.get_model_context_window(),
    });
    sess.send_event(&turn_context, start_event).await;
    run_compact_task_inner(sess.clone(), turn_context, input).await;
}

async fn run_compact_task_inner(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    input: Vec<UserInput>,
) {
    let initial_input_for_turn: ResponseInputItem = ResponseInputItem::from(input);

    let mut history = sess.clone_history().await;
    history.record_items(
        &[initial_input_for_turn.into()],
        turn_context.truncation_policy,
    );

    let mut truncated_count = 0usize;

    let max_retries = turn_context.client.get_provider().stream_max_retries();
    let mut retries = 0;

    let rollout_item = RolloutItem::TurnContext(TurnContextItem {
        cwd: turn_context.cwd.clone(),
        approval_policy: turn_context.approval_policy,
        sandbox_policy: turn_context.sandbox_policy.clone(),
        model: turn_context.client.get_model(),
        effort: turn_context.client.get_reasoning_effort(),
        summary: turn_context.client.get_reasoning_summary(),
    });
    sess.persist_rollout_items(&[rollout_item]).await;

    loop {
        let turn_input = history.get_history_for_prompt();
        let prompt = Prompt {
            input: turn_input.clone(),
            ..Default::default()
        };
        let attempt_result = drain_to_completed(&sess, turn_context.as_ref(), &prompt).await;

        match attempt_result {
            Ok(()) => {
                if truncated_count > 0 {
                    sess.notify_background_event(
                        turn_context.as_ref(),
                        format!(
                            "Trimmed {truncated_count} older conversation item(s) before compacting so the prompt fits the model context window."
                        ),
                    )
                    .await;
                }
                break;
            }
            Err(CodexErr::Interrupted) => {
                return;
            }
            Err(e @ CodexErr::ContextWindowExceeded) => {
                if turn_input.len() > 1 {
                    // Trim from the beginning to preserve cache (prefix-based) and keep recent messages intact.
                    error!(
                        "Context window exceeded while compacting; removing oldest history item. Error: {e}"
                    );
                    history.remove_first_item();
                    truncated_count += 1;
                    retries = 0;
                    continue;
                }
                sess.set_total_tokens_full(turn_context.as_ref()).await;
                let event = EventMsg::Error(e.to_error_event(None));
                sess.send_event(&turn_context, event).await;
                return;
            }
            Err(e) => {
                if retries < max_retries {
                    retries += 1;
                    let delay = backoff(retries);
                    sess.notify_stream_error(
                        turn_context.as_ref(),
                        format!("Reconnecting... {retries}/{max_retries}"),
                        e,
                    )
                    .await;
                    tokio::time::sleep(delay).await;
                    continue;
                } else {
                    let event = EventMsg::Error(e.to_error_event(None));
                    sess.send_event(&turn_context, event).await;
                    return;
                }
            }
        }
    }

    let history_snapshot = sess.clone_history().await.get_history();
    let summary_suffix =
        get_last_assistant_message_from_turn(&history_snapshot).unwrap_or_default();
    let summary_text = format!("{SUMMARY_PREFIX}\n{summary_suffix}");
    let user_messages = collect_user_messages(&history_snapshot);

    // 1. Build file restoration attachment BEFORE clearing read_file_state
    let file_restore_attachment = sess.build_compact_file_restore().await;

    // 2. Extract restored files info for TUI display
    let restored_files: Vec<RestoredFileInfo> = file_restore_attachment
        .as_ref()
        .map(|attachment| {
            if let ResponseItem::Attachment {
                data: AttachmentData::CompactFileRestore { files },
                ..
            } = attachment
            {
                files
                    .iter()
                    .map(|f| match f {
                        CompactRestoredFile::WithContent { path, num_lines, .. } => {
                            RestoredFileInfo {
                                path: path.clone(),
                                num_lines: Some(*num_lines),
                            }
                        }
                        CompactRestoredFile::ReferenceOnly { path } => RestoredFileInfo {
                            path: path.clone(),
                            num_lines: None,
                        },
                    })
                    .collect()
            } else {
                Vec::new()
            }
        })
        .unwrap_or_default();

    // 3. Clear read_file_state after building attachment
    sess.clear_read_file_state().await;

    // 4. Build compacted history with summary
    let initial_context = sess.build_initial_context(turn_context.as_ref());
    let mut new_history = build_compacted_history(initial_context, &user_messages, &summary_text);

    // 5. Preserve ghost snapshots
    let ghost_snapshots: Vec<ResponseItem> = history_snapshot
        .iter()
        .filter(|item| matches!(item, ResponseItem::GhostSnapshot { .. }))
        .cloned()
        .collect();
    new_history.extend(ghost_snapshots);

    // 6. Inject file restoration attachment into compacted history
    if let Some(attachment) = file_restore_attachment {
        new_history.push(attachment);
    }

    sess.replace_history(new_history).await;
    sess.recompute_token_usage(&turn_context).await;

    let rollout_item = RolloutItem::Compacted(CompactedItem {
        message: summary_text.clone(),
        replacement_history: None,
    });
    sess.persist_rollout_items(&[rollout_item]).await;

    let event = EventMsg::ContextCompacted(ContextCompactedEvent {
        restored_files,
        summary: Some(summary_text.clone()),
    });
    sess.send_event(&turn_context, event).await;

    let warning = EventMsg::Warning(WarningEvent {
        message: "Heads up: Long conversations and multiple compactions can cause the model to be less accurate. Start a new conversation when possible to keep conversations small and targeted.".to_string(),
    });
    sess.send_event(&turn_context, warning).await;
}

pub fn content_items_to_text(content: &[ContentItem]) -> Option<String> {
    let mut pieces = Vec::new();
    for item in content {
        match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                if !text.is_empty() {
                    pieces.push(text.as_str());
                }
            }
            ContentItem::InputImage { .. } => {}
        }
    }
    if pieces.is_empty() {
        None
    } else {
        Some(pieces.join("\n"))
    }
}

pub(crate) fn collect_user_messages(items: &[ResponseItem]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| match crate::event_mapping::parse_turn_item(item) {
            Some(TurnItem::UserMessage(user)) => {
                if is_summary_message(&user.message()) {
                    None
                } else {
                    Some(user.message())
                }
            }
            _ => None,
        })
        .collect()
}

pub(crate) fn is_summary_message(message: &str) -> bool {
    message.starts_with(format!("{SUMMARY_PREFIX}\n").as_str())
}

pub(crate) fn build_compacted_history(
    initial_context: Vec<ResponseItem>,
    user_messages: &[String],
    summary_text: &str,
) -> Vec<ResponseItem> {
    build_compacted_history_with_limit(
        initial_context,
        user_messages,
        summary_text,
        COMPACT_USER_MESSAGE_MAX_TOKENS,
    )
}

fn build_compacted_history_with_limit(
    mut history: Vec<ResponseItem>,
    user_messages: &[String],
    summary_text: &str,
    max_tokens: usize,
) -> Vec<ResponseItem> {
    let mut selected_messages: Vec<String> = Vec::new();
    if max_tokens > 0 {
        let mut remaining = max_tokens;
        for message in user_messages.iter().rev() {
            if remaining == 0 {
                break;
            }
            let tokens = approx_token_count(message);
            if tokens <= remaining {
                selected_messages.push(message.clone());
                remaining = remaining.saturating_sub(tokens);
            } else {
                let truncated = truncate_text(message, TruncationPolicy::Tokens(remaining));
                selected_messages.push(truncated);
                break;
            }
        }
        selected_messages.reverse();
    }

    for message in &selected_messages {
        history.push(ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: message.clone(),
            }],
        });
    }

    let summary_text = if summary_text.is_empty() {
        "(no summary available)".to_string()
    } else {
        summary_text.to_string()
    };

    history.push(ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText { text: summary_text }],
    });

    history
}

async fn drain_to_completed(
    sess: &Session,
    turn_context: &TurnContext,
    prompt: &Prompt,
) -> CodexResult<()> {
    let mut stream = turn_context.client.clone().stream(prompt).await?;
    loop {
        let maybe_event = stream.next().await;
        let Some(event) = maybe_event else {
            return Err(CodexErr::Stream(
                "stream closed before response.completed".into(),
                None,
            ));
        };
        match event {
            Ok(ResponseEvent::OutputItemDone(item)) => {
                sess.record_into_history(std::slice::from_ref(&item), turn_context)
                    .await;
            }
            Ok(ResponseEvent::RateLimits(snapshot)) => {
                sess.update_rate_limits(turn_context, snapshot).await;
            }
            Ok(ResponseEvent::Completed { token_usage, .. }) => {
                sess.update_token_usage_info(turn_context, token_usage.as_ref())
                    .await;
                return Ok(());
            }
            Ok(_) => continue,
            Err(e) => return Err(e),
        }
    }
}

/// Build CompactFileRestore attachment from read file state.
/// Called during compaction to restore recently read files.
pub(crate) fn build_compact_file_restore(read_state: &ReadFileState) -> Option<ResponseItem> {
    let recent_files = read_state.get_recent_files(MAX_RECENT_FILES_TO_RESTORE);

    if recent_files.is_empty() {
        return None;
    }

    let mut files = Vec::new();
    let mut total_tokens = 0usize;

    for (path, info) in recent_files {
        let content_tokens = approx_token_count(&info.content);
        let path_str = path.display().to_string();

        if content_tokens > MAX_TOKENS_PER_RESTORED_FILE {
            // Too large - reference only
            files.push(CompactRestoredFile::ReferenceOnly { path: path_str });
        } else if total_tokens + content_tokens <= MAX_TOTAL_RESTORE_TOKENS {
            // Within budget - include content
            let num_lines = info.content.lines().count();
            files.push(CompactRestoredFile::WithContent {
                path: path_str,
                content: info.content.clone(),
                num_lines,
                truncated: false,
            });
            total_tokens += content_tokens;
        } else {
            // Would exceed total budget - reference only
            files.push(CompactRestoredFile::ReferenceOnly { path: path_str });
        }
    }

    if files.is_empty() {
        return None;
    }

    Some(ResponseItem::Attachment {
        data: AttachmentData::CompactFileRestore { files },
        timestamp: chrono::Utc::now().to_rfc3339(),
    })
}

#[cfg(test)]
mod tests {

    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn content_items_to_text_joins_non_empty_segments() {
        let items = vec![
            ContentItem::InputText {
                text: "hello".to_string(),
            },
            ContentItem::OutputText {
                text: String::new(),
            },
            ContentItem::OutputText {
                text: "world".to_string(),
            },
        ];

        let joined = content_items_to_text(&items);

        assert_eq!(Some("hello\nworld".to_string()), joined);
    }

    #[test]
    fn content_items_to_text_ignores_image_only_content() {
        let items = vec![ContentItem::InputImage {
            image_url: "file://image.png".to_string(),
        }];

        let joined = content_items_to_text(&items);

        assert_eq!(None, joined);
    }

    #[test]
    fn collect_user_messages_extracts_user_text_only() {
        let items = vec![
            ResponseItem::Message {
                id: Some("assistant".to_string()),
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "ignored".to_string(),
                }],
            },
            ResponseItem::Message {
                id: Some("user".to_string()),
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "first".to_string(),
                }],
            },
            ResponseItem::Other,
        ];

        let collected = collect_user_messages(&items);

        assert_eq!(vec!["first".to_string()], collected);
    }

    #[test]
    fn collect_user_messages_filters_session_prefix_entries() {
        let items = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "# AGENTS.md instructions for project\n\n<INSTRUCTIONS>\ndo things\n</INSTRUCTIONS>"
                        .to_string(),
                }],
            },
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "<ENVIRONMENT_CONTEXT>cwd=/tmp</ENVIRONMENT_CONTEXT>".to_string(),
                }],
            },
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "real user message".to_string(),
                }],
            },
        ];

        let collected = collect_user_messages(&items);

        assert_eq!(vec!["real user message".to_string()], collected);
    }

    #[test]
    fn build_token_limited_compacted_history_truncates_overlong_user_messages() {
        // Use a small truncation limit so the test remains fast while still validating
        // that oversized user content is truncated.
        let max_tokens = 16;
        let big = "word ".repeat(200);
        let history = super::build_compacted_history_with_limit(
            Vec::new(),
            std::slice::from_ref(&big),
            "SUMMARY",
            max_tokens,
        );
        assert_eq!(history.len(), 2);

        let truncated_message = &history[0];
        let summary_message = &history[1];

        let truncated_text = match truncated_message {
            ResponseItem::Message { role, content, .. } if role == "user" => {
                content_items_to_text(content).unwrap_or_default()
            }
            other => panic!("unexpected item in history: {other:?}"),
        };

        assert!(
            truncated_text.contains("tokens truncated"),
            "expected truncation marker in truncated user message"
        );
        assert!(
            !truncated_text.contains(&big),
            "truncated user message should not include the full oversized user text"
        );

        let summary_text = match summary_message {
            ResponseItem::Message { role, content, .. } if role == "user" => {
                content_items_to_text(content).unwrap_or_default()
            }
            other => panic!("unexpected item in history: {other:?}"),
        };
        assert_eq!(summary_text, "SUMMARY");
    }

    #[test]
    fn build_token_limited_compacted_history_appends_summary_message() {
        let initial_context: Vec<ResponseItem> = Vec::new();
        let user_messages = vec!["first user message".to_string()];
        let summary_text = "summary text";

        let history = build_compacted_history(initial_context, &user_messages, summary_text);
        assert!(
            !history.is_empty(),
            "expected compacted history to include summary"
        );

        let last = history.last().expect("history should have a summary entry");
        let summary = match last {
            ResponseItem::Message { role, content, .. } if role == "user" => {
                content_items_to_text(content).unwrap_or_default()
            }
            other => panic!("expected summary message, found {other:?}"),
        };
        assert_eq!(summary, summary_text);
    }

    // Auto compact threshold tests

    /// Helper to clear the env var for testing (unsafe in Rust 2024 edition)
    fn clear_autocompact_env_var() {
        // SAFETY: Tests are single-threaded by default, and we don't spawn threads
        // that read this env var concurrently in these tests.
        unsafe {
            std::env::remove_var(CODEX_AUTOCOMPACT_PCT_OVERRIDE);
        }
    }

    #[test]
    fn test_get_auto_compact_threshold_default_60_percent() {
        // Remove env var to test default behavior
        clear_autocompact_env_var();

        let context_window = 100_000;
        let threshold = get_auto_compact_threshold(context_window, None, None);

        // Default is 60% minus buffer of 5000
        // 100_000 * 0.60 = 60_000, minus 5000 = 55_000
        assert_eq!(threshold, 55_000);
    }

    #[test]
    fn test_get_auto_compact_threshold_absolute_limit_priority() {
        // Remove env var to test config priority
        clear_autocompact_env_var();

        let context_window = 100_000;
        // Absolute limit should take priority over percentage config
        let threshold =
            get_auto_compact_threshold(context_window, Some(80_000), Some(50)); // 50% would be 45_000

        assert_eq!(threshold, 80_000);
    }

    #[test]
    fn test_get_auto_compact_threshold_pct_config() {
        // Remove env var to test config priority
        clear_autocompact_env_var();

        let context_window = 100_000;
        // With percentage config, no absolute limit
        let threshold = get_auto_compact_threshold(context_window, None, Some(80));

        // 80% of 100_000 = 80_000, minus 5000 buffer = 75_000
        assert_eq!(threshold, 75_000);
    }

    #[test]
    fn test_should_auto_compact_enabled() {
        clear_autocompact_env_var();

        // Just above threshold
        let above = should_auto_compact(56_000, 100_000, None, None, true);
        assert!(above, "should trigger when above threshold");

        // Just below threshold (55_000 is the threshold at 60% - 5000 buffer)
        let below = should_auto_compact(54_000, 100_000, None, None, true);
        assert!(!below, "should not trigger when below threshold");
    }

    #[test]
    fn test_should_auto_compact_disabled() {
        clear_autocompact_env_var();

        // Even above threshold, should not trigger when disabled
        let result = should_auto_compact(90_000, 100_000, None, None, false);
        assert!(!result, "should not trigger when disabled");
    }

    #[test]
    fn test_threshold_priority_order() {
        // Test that priority order is: env var > absolute limit > pct config > default
        clear_autocompact_env_var();

        let context_window = 100_000;

        // Priority 4: Default (60%)
        let default_threshold = get_auto_compact_threshold(context_window, None, None);
        assert_eq!(default_threshold, 55_000); // 60% - 5000

        // Priority 3: pct config
        let pct_threshold = get_auto_compact_threshold(context_window, None, Some(70));
        assert_eq!(pct_threshold, 65_000); // 70% - 5000

        // Priority 2: absolute limit
        let abs_threshold = get_auto_compact_threshold(context_window, Some(50_000), Some(70));
        assert_eq!(abs_threshold, 50_000); // absolute overrides pct
    }
}
