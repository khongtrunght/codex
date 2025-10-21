#![cfg(not(target_os = "windows"))]

use std::fs;

use codex_core::model_family::find_family_for_model;
use codex_core::protocol::AskForApproval;
use codex_core::protocol::EventMsg;
use codex_core::protocol::InputItem;
use codex_core::protocol::Op;
use codex_core::protocol::SandboxPolicy;
use codex_protocol::config_types::ReasoningSummary;
use core_test_support::responses;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_function_call;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::TestCodex;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use serde_json::Value;
use serde_json::json;

fn extract_output_text(item: &Value) -> Option<&str> {
    item.get("output").and_then(|value| match value {
        Value::String(text) => Some(text.as_str()),
        Value::Object(obj) => obj.get("content").and_then(Value::as_str),
        _ => None,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_file_tool_executes_and_emits_file_edit_events() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;

    let mut builder = test_codex().with_config(|config| {
        config.model = "claude-3-5-sonnet-20241022".to_string();
        config.model_family =
            find_family_for_model("claude-3-5-sonnet-20241022").expect("valid model");
    });
    let TestCodex {
        codex,
        cwd,
        session_configured,
        ..
    } = builder.build(&server).await?;

    let file_name = "test_write.txt";
    let file_path = cwd.path().join(file_name);
    let call_id = "write-file-call";
    let write_args = json!({
        "file_path": file_name,
        "file_content": "Test content from write_file tool\n",
    })
    .to_string();

    let first_response = sse(vec![
        ev_response_created("resp-1"),
        ev_function_call(call_id, "write_file", &write_args),
        ev_completed("resp-1"),
    ]);
    responses::mount_sse_once_match(&server, wiremock::matchers::any(), first_response).await;

    let second_response = sse(vec![
        ev_assistant_message("msg-1", "file written"),
        ev_completed("resp-2"),
    ]);
    let second_mock =
        responses::mount_sse_once_match(&server, wiremock::matchers::any(), second_response).await;

    let session_model = session_configured.model.clone();

    codex
        .submit(Op::UserTurn {
            items: vec![InputItem::Text {
                text: "please write a file".into(),
            }],
            final_output_json_schema: None,
            cwd: cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: session_model,
            effort: None,
            summary: ReasoningSummary::Auto,
        })
        .await?;

    let mut saw_file_edit_begin = false;
    let mut file_edit_end_success = None;
    wait_for_event(&codex, |event| match event {
        EventMsg::FileEditBegin(begin) => {
            saw_file_edit_begin = true;
            assert_eq!(begin.call_id, call_id);
            assert!(begin.auto_approved);
            // Verify the FileChange contains the Add variant
            assert_eq!(begin.changes.len(), 1);
            false
        }
        EventMsg::FileEditEnd(end) => {
            assert_eq!(end.call_id, call_id);
            file_edit_end_success = Some(end.success);
            false
        }
        EventMsg::TaskComplete(_) => true,
        _ => false,
    })
    .await;

    assert!(saw_file_edit_begin, "expected FileEditBegin event");
    let file_edit_end_success =
        file_edit_end_success.expect("expected FileEditEnd event to capture success flag");
    assert!(file_edit_end_success);

    let req = second_mock.single_request();
    let output_item = req.function_call_output(call_id);
    assert_eq!(
        output_item.get("call_id").and_then(Value::as_str),
        Some(call_id)
    );

    // Verify file was actually written
    let updated_contents = fs::read_to_string(file_path)?;
    assert_eq!(
        updated_contents, "Test content from write_file tool\n",
        "expected file to be written with correct content"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn edit_file_tool_executes_and_emits_file_edit_events() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;

    let mut builder = test_codex().with_config(|config| {
        config.model = "claude-3-5-sonnet-20241022".to_string();
        config.model_family =
            find_family_for_model("claude-3-5-sonnet-20241022").expect("valid model");
    });
    let TestCodex {
        codex,
        cwd,
        session_configured,
        ..
    } = builder.build(&server).await?;

    let file_name = "test_edit.txt";
    let file_path = cwd.path().join(file_name);

    // Create initial file
    fs::write(&file_path, "Original content\n")?;

    let call_id = "edit-file-call";
    let edit_args = json!({
        "file_path": file_name,
        "old_string": "Original",
        "new_string": "Modified",
    })
    .to_string();

    let first_response = sse(vec![
        ev_response_created("resp-1"),
        ev_function_call(call_id, "edit_file", &edit_args),
        ev_completed("resp-1"),
    ]);
    responses::mount_sse_once_match(&server, wiremock::matchers::any(), first_response).await;

    let second_response = sse(vec![
        ev_assistant_message("msg-1", "file edited"),
        ev_completed("resp-2"),
    ]);
    let second_mock =
        responses::mount_sse_once_match(&server, wiremock::matchers::any(), second_response).await;

    let session_model = session_configured.model.clone();

    codex
        .submit(Op::UserTurn {
            items: vec![InputItem::Text {
                text: "please edit a file".into(),
            }],
            final_output_json_schema: None,
            cwd: cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: session_model,
            effort: None,
            summary: ReasoningSummary::Auto,
        })
        .await?;

    let mut saw_file_edit_begin = false;
    let mut file_edit_end_success = None;
    wait_for_event(&codex, |event| match event {
        EventMsg::FileEditBegin(begin) => {
            saw_file_edit_begin = true;
            assert_eq!(begin.call_id, call_id);
            assert!(begin.auto_approved);
            // Verify the FileChange contains the Update variant
            assert_eq!(begin.changes.len(), 1);
            false
        }
        EventMsg::FileEditEnd(end) => {
            assert_eq!(end.call_id, call_id);
            file_edit_end_success = Some(end.success);
            false
        }
        EventMsg::TaskComplete(_) => true,
        _ => false,
    })
    .await;

    assert!(saw_file_edit_begin, "expected FileEditBegin event");
    let file_edit_end_success =
        file_edit_end_success.expect("expected FileEditEnd event to capture success flag");
    assert!(file_edit_end_success);

    let req = second_mock.single_request();
    let output_item = req.function_call_output(call_id);
    assert_eq!(
        output_item.get("call_id").and_then(Value::as_str),
        Some(call_id)
    );

    // Verify file was actually edited
    let updated_contents = fs::read_to_string(file_path)?;
    assert_eq!(
        updated_contents, "Modified content\n",
        "expected file to be edited with correct content"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_file_reports_errors_in_file_edit_end_event() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;

    let mut builder = test_codex().with_config(|config| {
        config.model = "claude-3-5-sonnet-20241022".to_string();
        config.model_family =
            find_family_for_model("claude-3-5-sonnet-20241022").expect("valid model");
    });
    let TestCodex {
        codex,
        cwd,
        session_configured,
        ..
    } = builder.build(&server).await?;

    let call_id = "write-file-error-call";
    // Try to write to an invalid path (containing null byte)
    let write_args = json!({
        "file_path": "/invalid/\0/path.txt",
        "file_content": "Test content",
    })
    .to_string();

    let first_response = sse(vec![
        ev_response_created("resp-1"),
        ev_function_call(call_id, "write_file", &write_args),
        ev_completed("resp-1"),
    ]);
    responses::mount_sse_once_match(&server, wiremock::matchers::any(), first_response).await;

    let second_response = sse(vec![
        ev_assistant_message("msg-1", "failed"),
        ev_completed("resp-2"),
    ]);
    let second_mock =
        responses::mount_sse_once_match(&server, wiremock::matchers::any(), second_response).await;

    let session_model = session_configured.model.clone();

    codex
        .submit(Op::UserTurn {
            items: vec![InputItem::Text {
                text: "please write a file".into(),
            }],
            final_output_json_schema: None,
            cwd: cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: session_model,
            effort: None,
            summary: ReasoningSummary::Auto,
        })
        .await?;

    let mut saw_file_edit_end = false;
    let mut file_edit_end_success = None;
    wait_for_event(&codex, |event| match event {
        EventMsg::FileEditEnd(end) => {
            saw_file_edit_end = true;
            assert_eq!(end.call_id, call_id);
            file_edit_end_success = Some(end.success);
            assert!(!end.stderr.is_empty(), "expected error message in stderr");
            false
        }
        EventMsg::TaskComplete(_) => true,
        _ => false,
    })
    .await;

    assert!(saw_file_edit_end, "expected FileEditEnd event");
    let file_edit_end_success = file_edit_end_success.expect("expected FileEditEnd event");
    assert!(!file_edit_end_success, "expected failure for invalid path");

    let req = second_mock.single_request();
    let output_item = req.function_call_output(call_id);
    let output_text = extract_output_text(&output_item).expect("output text present");

    assert!(
        output_text.contains("Failed") || output_text.contains("failed"),
        "expected error message in output text, got {output_text:?}"
    );

    if let Some(success_flag) = output_item
        .get("output")
        .and_then(|value| value.as_object())
        .and_then(|obj| obj.get("success"))
        .and_then(serde_json::Value::as_bool)
    {
        assert!(
            !success_flag,
            "expected tool output to mark success=false for write failures"
        );
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn edit_file_replace_all_executes() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;

    let mut builder = test_codex().with_config(|config| {
        config.model = "claude-3-5-sonnet-20241022".to_string();
        config.model_family =
            find_family_for_model("claude-3-5-sonnet-20241022").expect("valid model");
    });
    let TestCodex {
        codex,
        cwd,
        session_configured,
        ..
    } = builder.build(&server).await?;

    let file_name = "test_replace_all.txt";
    let file_path = cwd.path().join(file_name);

    // Create file with repeated text
    let initial_content = "hello\nworld\nhello\nuniverse\nhello\n";
    fs::write(&file_path, initial_content)?;

    let call_id = "replace-all-call";
    let edit_args = json!({
        "file_path": file_name,
        "old_string": "hello",
        "new_string": "goodbye",
        "replace_all": true,
    })
    .to_string();

    let first_response = sse(vec![
        ev_response_created("resp-1"),
        ev_function_call(call_id, "edit_file", &edit_args),
        ev_completed("resp-1"),
    ]);
    responses::mount_sse_once_match(&server, wiremock::matchers::any(), first_response).await;

    let second_response = sse(vec![
        ev_assistant_message("msg-1", "file edited"),
        ev_completed("resp-2"),
    ]);
    let second_mock =
        responses::mount_sse_once_match(&server, wiremock::matchers::any(), second_response).await;

    let session_model = session_configured.model.clone();

    codex
        .submit(Op::UserTurn {
            items: vec![InputItem::Text {
                text: "Replace all hello with goodbye".into(),
            }],
            final_output_json_schema: None,
            cwd: cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: session_model,
            effort: None,
            summary: ReasoningSummary::Auto,
        })
        .await?;

    // Verify FileEditEnd event shows success
    let mut file_edit_end_success = None;
    let mut file_edit_end_stderr = String::new();
    wait_for_event(&codex, |event| match event {
        EventMsg::FileEditEnd(end) => {
            assert_eq!(end.call_id, call_id);
            file_edit_end_success = Some(end.success);
            file_edit_end_stderr = end.stderr.clone();
            false
        }
        EventMsg::TaskComplete(_) => true,
        _ => false,
    })
    .await;

    let file_edit_end_success = file_edit_end_success.expect("expected FileEditEnd event");
    assert!(file_edit_end_success, "expected success");
    assert_eq!(file_edit_end_stderr, "", "expected no error");

    // Verify file was updated with all occurrences replaced
    let final_content = fs::read_to_string(&file_path)?;
    assert_eq!(
        final_content, "goodbye\nworld\ngoodbye\nuniverse\ngoodbye\n",
        "expected all occurrences of hello to be replaced"
    );

    // Verify tool output
    let req = second_mock.single_request();
    let output_item = req.function_call_output(call_id);
    let output_text = extract_output_text(&output_item).expect("output text present");
    assert!(
        output_text.contains("Successfully replaced"),
        "expected success message in output"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn edit_file_without_replace_all_fails_on_multiple() -> anyhow::Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;

    let mut builder = test_codex().with_config(|config| {
        config.model = "claude-3-5-sonnet-20241022".to_string();
        config.model_family =
            find_family_for_model("claude-3-5-sonnet-20241022").expect("valid model");
    });
    let TestCodex {
        codex,
        cwd,
        session_configured,
        ..
    } = builder.build(&server).await?;

    let file_name = "test_unique.txt";
    let file_path = cwd.path().join(file_name);

    // Create file with repeated text
    let initial_content = "test\ntest\n";
    fs::write(&file_path, initial_content)?;

    let call_id = "unique-fail-call";
    // Try to edit without replace_all (should fail)
    let edit_args = json!({
        "file_path": file_name,
        "old_string": "test",
        "new_string": "hello",
    })
    .to_string();

    let first_response = sse(vec![
        ev_response_created("resp-1"),
        ev_function_call(call_id, "edit_file", &edit_args),
        ev_completed("resp-1"),
    ]);
    responses::mount_sse_once_match(&server, wiremock::matchers::any(), first_response).await;

    let second_response = sse(vec![
        ev_assistant_message("msg-1", "failed"),
        ev_completed("resp-2"),
    ]);
    responses::mount_sse_once_match(&server, wiremock::matchers::any(), second_response).await;

    let session_model = session_configured.model.clone();

    codex
        .submit(Op::UserTurn {
            items: vec![InputItem::Text {
                text: "Replace test with hello".into(),
            }],
            final_output_json_schema: None,
            cwd: cwd.path().to_path_buf(),
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            model: session_model,
            effort: None,
            summary: ReasoningSummary::Auto,
        })
        .await?;

    // Verify FileEditEnd event shows failure
    let mut file_edit_end_success = None;
    let mut file_edit_end_stderr = String::new();
    wait_for_event(&codex, |event| match event {
        EventMsg::FileEditEnd(end) => {
            assert_eq!(end.call_id, call_id);
            file_edit_end_success = Some(end.success);
            file_edit_end_stderr = end.stderr.clone();
            false
        }
        EventMsg::TaskComplete(_) => true,
        _ => false,
    })
    .await;

    let file_edit_end_success =
        file_edit_end_success.expect("expected FileEditEnd event to capture success flag");
    assert!(!file_edit_end_success, "expected failure");
    assert!(
        file_edit_end_stderr.contains("appears 2 times"),
        "expected error to mention occurrence count, got: {file_edit_end_stderr}"
    );
    assert!(
        file_edit_end_stderr.contains("replace_all: true"),
        "expected error to suggest replace_all option, got: {file_edit_end_stderr}"
    );

    // Verify file was NOT modified
    let final_content = fs::read_to_string(&file_path)?;
    assert_eq!(
        final_content, initial_content,
        "expected file to remain unchanged after failed edit"
    );

    Ok(())
}
