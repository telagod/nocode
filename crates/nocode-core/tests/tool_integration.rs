//! Integration tests for tool execution end-to-end paths.
//!
//! Covers: file tool roundtrips, bash execution, glob/grep, security boundaries,
//! tool input validation, and mock parity scenarios.

use std::fs;

use nocode_core::tool_execution::{
    DefaultToolExecutor, LiveToolHost, ToolCallInput, ToolCallResult, ToolExecutionContext,
    ToolExecutionRequest, ToolExecutor,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn live_executor_in(dir: &std::path::Path) -> DefaultToolExecutor<LiveToolHost> {
    DefaultToolExecutor {
        context: ToolExecutionContext::new(dir.to_string_lossy().into_owned()),
        host: LiveToolHost,
    }
}

fn run_allowed(
    executor: &DefaultToolExecutor<LiveToolHost>,
    call: ToolCallInput,
) -> ToolCallResult {
    executor.execute(ToolExecutionRequest::allowed(call)).result
}

// ---------------------------------------------------------------------------
// a) Tool execution end-to-end
// ---------------------------------------------------------------------------

#[test]
fn read_edit_write_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Write a seed file on disk.
    fs::write(dir.join("hello.txt"), "alpha beta gamma").unwrap();

    let exec = live_executor_in(dir);

    // 1. Read
    let result = run_allowed(
        &exec,
        ToolCallInput::new("Read", "t-1").with_argument("file_path", "hello.txt"),
    );
    assert_eq!(result.status_label(), "completed");
    assert!(result.message().contains("read"));

    // 2. Edit — replace "beta" with "REPLACED"
    let result = run_allowed(
        &exec,
        ToolCallInput::new("Edit", "t-2")
            .with_argument("file_path", "hello.txt")
            .with_argument("old_string", "beta")
            .with_argument("new_string", "REPLACED"),
    );
    assert_eq!(result.status_label(), "completed");
    assert!(result.message().contains("edited"));

    // 3. Read again — verify edit took effect
    let result = run_allowed(
        &exec,
        ToolCallInput::new("Read", "t-3").with_argument("file_path", "hello.txt"),
    );
    assert_eq!(result.status_label(), "completed");
    // The generated message should contain the updated content.
    if let ToolCallResult::Completed { output, .. } = &result {
        let msg_text = output
            .generated_messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("");
        assert!(
            msg_text.contains("REPLACED"),
            "edit should be visible in re-read"
        );
        assert!(!msg_text.contains("beta"), "old text should be gone");
    } else {
        panic!("expected Completed");
    }

    // 4. Write a new file
    let result = run_allowed(
        &exec,
        ToolCallInput::new("Write", "t-4")
            .with_argument("file_path", "new_file.txt")
            .with_argument("content", "fresh content"),
    );
    assert_eq!(result.status_label(), "completed");
    assert_eq!(
        fs::read_to_string(dir.join("new_file.txt")).unwrap(),
        "fresh content"
    );
}

#[test]
fn bash_captures_stdout_and_stderr() {
    let tmp = tempfile::tempdir().unwrap();
    let exec = live_executor_in(tmp.path());

    let result = run_allowed(
        &exec,
        ToolCallInput::new("Bash", "t-5").with_argument("command", "echo hello_world"),
    );
    assert_eq!(result.status_label(), "completed");
    if let ToolCallResult::Completed { output, .. } = &result {
        let msg_text = output
            .generated_messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("");
        assert!(
            msg_text.contains("hello_world"),
            "stdout should be captured"
        );
    } else {
        panic!("expected Completed");
    }
}
#[test]
fn glob_finds_files_in_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // Create a small directory tree.
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/main.rs"), "fn main() {}").unwrap();
    fs::write(dir.join("src/lib.rs"), "pub mod foo;").unwrap();
    fs::write(dir.join("README.md"), "# hello").unwrap();

    let exec = live_executor_in(dir);

    let result = run_allowed(
        &exec,
        ToolCallInput::new("Glob", "t-6").with_argument("pattern", "*/src/*.rs"),
    );
    assert_eq!(result.status_label(), "completed");
    if let ToolCallResult::Completed { output, .. } = &result {
        let msg_text = output
            .generated_messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("");
        assert!(msg_text.contains("main.rs"), "glob should find main.rs");
        assert!(msg_text.contains("lib.rs"), "glob should find lib.rs");
        assert!(
            !msg_text.contains("README.md"),
            "glob should not match README.md"
        );
    } else {
        panic!("expected Completed");
    }
}

#[test]
fn grep_finds_pattern_in_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    fs::write(dir.join("a.txt"), "hello world\ngoodbye world\n").unwrap();
    fs::write(dir.join("b.txt"), "no match here\n").unwrap();

    let exec = live_executor_in(dir);

    let result = run_allowed(
        &exec,
        ToolCallInput::new("Grep", "t-7")
            .with_argument("pattern", "hello")
            .with_argument("path", dir.to_string_lossy().as_ref()),
    );
    assert_eq!(result.status_label(), "completed");
    if let ToolCallResult::Completed { output, .. } = &result {
        let msg_text = output
            .generated_messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("");
        assert!(
            msg_text.contains("hello world"),
            "grep should find the pattern"
        );
        assert!(
            !msg_text.contains("no match"),
            "grep should not include non-matching lines"
        );
    } else {
        panic!("expected Completed");
    }
}

// ---------------------------------------------------------------------------
// b) Security boundaries
// ---------------------------------------------------------------------------

#[test]
fn read_rejects_path_outside_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    let exec = live_executor_in(tmp.path());

    let result = run_allowed(
        &exec,
        ToolCallInput::new("Read", "t-8").with_argument("file_path", "/etc/passwd"),
    );
    assert!(
        matches!(result, ToolCallResult::Failed { .. }),
        "reading /etc/passwd should fail: got {}",
        result.status_label()
    );
}

#[test]
fn write_rejects_path_outside_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    let exec = live_executor_in(tmp.path());

    // Use a path clearly outside the tempdir.
    let outside = tempfile::tempdir().unwrap();
    let evil_path = outside.path().join("evil.txt");

    let result = run_allowed(
        &exec,
        ToolCallInput::new("Write", "t-9")
            .with_argument("file_path", evil_path.to_string_lossy().as_ref())
            .with_argument("content", "pwned"),
    );
    assert!(
        matches!(result, ToolCallResult::Failed { .. }),
        "writing outside cwd should fail: got {}",
        result.status_label()
    );
    // File should not have been created.
    assert!(!evil_path.exists(), "file outside cwd must not be created");
}

#[test]
fn bash_blocks_destructive_commands() {
    let tmp = tempfile::tempdir().unwrap();
    let exec = live_executor_in(tmp.path());

    let dangerous = [
        "rm -rf /",
        "mkfs.ext4 /dev/sda1",
        "shutdown -h now",
        "reboot",
    ];
    for cmd in &dangerous {
        let result = run_allowed(
            &exec,
            ToolCallInput::new("Bash", "t-10").with_argument("command", *cmd),
        );
        assert!(
            matches!(result, ToolCallResult::Failed { .. }),
            "destructive command '{cmd}' should be blocked, got {}",
            result.status_label()
        );
    }
}

#[test]
fn bash_allows_safe_commands() {
    let tmp = tempfile::tempdir().unwrap();
    let exec = live_executor_in(tmp.path());

    let safe = ["ls -la", "echo safe", "pwd"];
    for cmd in &safe {
        let result = run_allowed(
            &exec,
            ToolCallInput::new("Bash", "t-11").with_argument("command", *cmd),
        );
        assert_eq!(
            result.status_label(),
            "completed",
            "safe command '{cmd}' should succeed"
        );
    }
}

// ---------------------------------------------------------------------------
// c) Tool validation
// ---------------------------------------------------------------------------

#[test]
fn missing_required_argument_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let exec = live_executor_in(tmp.path());

    // Read without file_path — schema validation should reject.
    let result = run_allowed(&exec, ToolCallInput::new("Read", "t-12"));
    assert!(
        matches!(result, ToolCallResult::Failed { .. }),
        "Read without file_path should fail"
    );
    if let ToolCallResult::Failed { error, .. } = &result {
        assert!(
            error.contains("file_path") || error.contains("validation"),
            "error should mention missing field: {error}"
        );
    }
}

#[test]
fn edit_with_nonexistent_old_string_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    fs::write(dir.join("target.txt"), "actual content here").unwrap();

    let exec = live_executor_in(dir);

    let result = run_allowed(
        &exec,
        ToolCallInput::new("Edit", "t-13")
            .with_argument("file_path", "target.txt")
            .with_argument("old_string", "DOES_NOT_EXIST_IN_FILE")
            .with_argument("new_string", "replacement"),
    );
    assert!(
        matches!(result, ToolCallResult::Failed { .. }),
        "edit with nonexistent old_string should fail"
    );
    if let ToolCallResult::Failed { error, .. } = &result {
        assert!(
            error.contains("not found"),
            "error should mention text not found: {error}"
        );
    }
    // File should be unchanged.
    assert_eq!(
        fs::read_to_string(dir.join("target.txt")).unwrap(),
        "actual content here"
    );
}

// ---------------------------------------------------------------------------
// d) Mock parity
// ---------------------------------------------------------------------------

#[test]
fn query_engine_executes_tool_calls_from_mock_model() {
    use nocode_core::mock_service::ParityTestRunner;

    let mut runner = ParityTestRunner::new();

    // bash_stdout_roundtrip triggers a Bash tool_use block.
    let result = runner.run_scenario("bash_stdout_roundtrip").unwrap();
    runner.assert_scenario_passed(&result);

    // The response should contain a tool_use for Bash.
    assert!(
        result.captured[0].body.contains("bash_stdout"),
        "request body should contain the trigger"
    );
}

#[test]
fn query_engine_respects_max_turns() {
    use nocode_core::mock_service::ParityTestRunner;

    let mut runner = ParityTestRunner::new();

    // Run a simple text scenario — should complete in 1 request.
    let result = runner.run_scenario("streaming_text").unwrap();
    assert_eq!(
        result.request_count, 1,
        "single-turn scenario should use exactly 1 request"
    );
    assert!(result.response_matched);
}

// ---------------------------------------------------------------------------
// e) Denied execution trace
// ---------------------------------------------------------------------------

#[test]
fn denied_request_produces_denied_trace() {
    let tmp = tempfile::tempdir().unwrap();
    let exec = live_executor_in(tmp.path());

    let trace = exec.execute(ToolExecutionRequest::denied(
        ToolCallInput::new("Write", "t-14")
            .with_argument("file_path", "test.txt")
            .with_argument("content", "data"),
        "permission denied by policy",
    ));

    assert_eq!(trace.result.status_label(), "denied");
    assert_eq!(
        trace.permission_denial.as_deref(),
        Some("permission denied by policy")
    );
}

// ---------------------------------------------------------------------------
// f) Write creates intermediate directories
// ---------------------------------------------------------------------------

#[test]
fn write_creates_parent_directories() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let exec = live_executor_in(dir);

    let result = run_allowed(
        &exec,
        ToolCallInput::new("Write", "t-15")
            .with_argument("file_path", "deep/nested/file.txt")
            .with_argument("content", "nested content"),
    );
    assert_eq!(result.status_label(), "completed");
    assert_eq!(
        fs::read_to_string(dir.join("deep/nested/file.txt")).unwrap(),
        "nested content"
    );
}
