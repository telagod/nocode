//! Integration test: tool execution roundtrips.
//!
//! Tests the full pipeline: ToolExecutor → validate → permission → execute → result.

use nocode_core::message::ContentBlock;
use nocode_core::tool::ToolRegistry;
use nocode_core::tool::executor::ToolExecutor;
use nocode_core::tool::hook_runner::HookRunner;
use nocode_core::tool::permission::PermissionMode;
use nocode_core::tool::trust::PermissionEnforcer;
use serde_json::json;

fn registry() -> ToolRegistry {
    ToolRegistry::with_defaults("/tmp")
}

fn assert_tool_success(result: &ContentBlock) {
    if let ContentBlock::ToolResult { is_error, .. } = result {
        assert!(!is_error, "Expected success but got error: {result:?}");
    } else {
        panic!("Expected ToolResult, got {result:?}");
    }
}

fn assert_tool_error(result: &ContentBlock, contains: &str) {
    if let ContentBlock::ToolResult {
        content, is_error, ..
    } = result
    {
        assert!(is_error, "Expected error but got success");
        assert!(
            content.contains(contains),
            "Expected error containing '{contains}', got: {content}"
        );
    } else {
        panic!("Expected ToolResult, got {result:?}");
    }
}

// ---------------------------------------------------------------------------
// Basic tool execution
// ---------------------------------------------------------------------------

#[test]
fn bash_echo_roundtrip() {
    let reg = registry();
    let exec = ToolExecutor::new(&reg);
    let result = exec.execute_tool_use("t1", "Bash", &json!({"command": "echo roundtrip"}));
    assert_tool_success(&result);
    if let ContentBlock::ToolResult { content, .. } = &result {
        assert!(content.contains("roundtrip"));
    }
}

#[test]
fn read_nonexistent_file_returns_error() {
    let reg = registry();
    let exec = ToolExecutor::new(&reg);
    let result = exec.execute_tool_use(
        "t2",
        "Read",
        &json!({"file_path": "/tmp/nocode_test_nonexistent_xyz_12345"}),
    );
    assert_tool_error(&result, "");
}

#[test]
fn glob_finds_files() {
    let reg = registry();
    let exec = ToolExecutor::new(&reg);
    let result = exec.execute_tool_use("t3", "Glob", &json!({"pattern": "/tmp/*"}));
    // Glob should succeed even if /tmp has files or not
    assert_tool_success(&result);
}

#[test]
fn grep_searches_content() {
    // Create a temp file to grep
    std::fs::write("/tmp/nocode_grep_test.txt", "hello world\nfoo bar\n").unwrap();
    let reg = registry();
    let exec = ToolExecutor::new(&reg);
    let result = exec.execute_tool_use(
        "t4",
        "Grep",
        &json!({"pattern": "hello", "path": "/tmp/nocode_grep_test.txt"}),
    );
    assert_tool_success(&result);
    if let ContentBlock::ToolResult { content, .. } = &result {
        assert!(content.contains("hello"));
    }
    let _ = std::fs::remove_file("/tmp/nocode_grep_test.txt");
}

#[test]
fn write_and_read_roundtrip() {
    let path = "/tmp/nocode_write_test.txt";
    let reg = registry();
    let exec = ToolExecutor::new(&reg);

    let write_result = exec.execute_tool_use(
        "t5w",
        "Write",
        &json!({"file_path": path, "content": "test content 123"}),
    );
    assert_tool_success(&write_result);

    let read_result = exec.execute_tool_use("t5r", "Read", &json!({"file_path": path}));
    assert_tool_success(&read_result);
    if let ContentBlock::ToolResult { content, .. } = &read_result {
        assert!(content.contains("test content 123"));
    }
    let _ = std::fs::remove_file(path);
}

#[test]
fn edit_replaces_text() {
    let path = "/tmp/nocode_edit_test.txt";
    std::fs::write(path, "old text here\n").unwrap();
    let reg = registry();
    let exec = ToolExecutor::new(&reg);

    let result = exec.execute_tool_use(
        "t6",
        "Edit",
        &json!({
            "file_path": path,
            "old_string": "old text",
            "new_string": "new text"
        }),
    );
    assert_tool_success(&result);

    let content = std::fs::read_to_string(path).unwrap();
    assert!(content.contains("new text"));
    assert!(!content.contains("old text"));
    let _ = std::fs::remove_file(path);
}

// ---------------------------------------------------------------------------
// Permission modes
// ---------------------------------------------------------------------------

#[test]
fn deny_mode_blocks_everything() {
    let reg = registry();
    let exec = ToolExecutor::new(&reg).with_permission_mode(PermissionMode::Deny);
    let result = exec.execute_tool_use("t7", "Bash", &json!({"command": "echo denied"}));
    assert_tool_error(&result, "Permission denied");
}

#[test]
fn ask_mode_allows_read_tools() {
    let reg = registry();
    let exec = ToolExecutor::new(&reg).with_permission_mode(PermissionMode::Ask);
    let result = exec.execute_tool_use("t8", "Glob", &json!({"pattern": "/tmp/*"}));
    assert_tool_success(&result);
}

// ---------------------------------------------------------------------------
// Trust system integration
// ---------------------------------------------------------------------------

#[test]
fn trust_allow_all_passes() {
    let reg = registry();
    let enforcer = PermissionEnforcer::allow_all();
    let exec = ToolExecutor::new(&reg).with_trust(enforcer);
    let result = exec.execute_tool_use("t9", "Bash", &json!({"command": "echo trust"}));
    assert_tool_success(&result);
}

// ---------------------------------------------------------------------------
// Hook integration
// ---------------------------------------------------------------------------

#[test]
fn pre_hook_can_deny_tool() {
    let reg = registry();
    let runner = HookRunner::new(
        vec![nocode_core::config::runtime::HookEntry {
            command: "exit 1".to_string(),
            tool_filter: None,
            timeout_ms: None,
        }],
        Vec::new(),
        Vec::new(),
    );
    let exec = ToolExecutor::new(&reg).with_hooks(&runner);
    let result = exec.execute_tool_use("t10", "Bash", &json!({"command": "echo hooked"}));
    assert_tool_error(&result, "PreToolUse hook denied");
}

#[test]
fn pre_hook_allows_on_success() {
    let reg = registry();
    let runner = HookRunner::new(
        vec![nocode_core::config::runtime::HookEntry {
            command: "true".to_string(),
            tool_filter: None,
            timeout_ms: None,
        }],
        Vec::new(),
        Vec::new(),
    );
    let exec = ToolExecutor::new(&reg).with_hooks(&runner);
    let result = exec.execute_tool_use("t11", "Bash", &json!({"command": "echo allowed"}));
    assert_tool_success(&result);
}

// ---------------------------------------------------------------------------
// Sandbox integration
// ---------------------------------------------------------------------------

#[test]
fn sandbox_blocks_network_tools() {
    let reg = registry();
    let sandbox = nocode_core::config::runtime::SandboxConfig {
        enabled: true,
        allowed_paths: vec!["/tmp".to_string()],
        network_enabled: false,
    };
    let exec = ToolExecutor::new(&reg).with_sandbox(sandbox);
    let result = exec.execute_tool_use("t12", "WebFetch", &json!({"url": "https://example.com"}));
    assert_tool_error(&result, "network access disabled");
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

#[test]
fn missing_required_field_rejected() {
    let reg = registry();
    let exec = ToolExecutor::new(&reg);
    let result = exec.execute_tool_use("t13", "Bash", &json!({}));
    assert_tool_error(&result, "Validation error");
}

#[test]
fn destructive_bash_blocked() {
    let reg = registry();
    let exec = ToolExecutor::new(&reg);
    let result = exec.execute_tool_use("t14", "Bash", &json!({"command": "rm -rf /"}));
    assert_tool_error(&result, "Bash validation");
}

#[test]
fn unknown_tool_rejected() {
    let reg = registry();
    let exec = ToolExecutor::new(&reg);
    let result = exec.execute_tool_use("t15", "FakeTool", &json!({}));
    assert_tool_error(&result, "not found");
}

// ---------------------------------------------------------------------------
// Execute all
// ---------------------------------------------------------------------------

#[test]
fn execute_all_processes_multiple_tools() {
    let reg = registry();
    let exec = ToolExecutor::new(&reg);
    let blocks = vec![
        ContentBlock::ToolUse {
            id: "a1".to_string(),
            name: "Bash".to_string(),
            input: json!({"command": "echo first"}),
        },
        ContentBlock::Text {
            text: "ignored".to_string(),
        },
        ContentBlock::ToolUse {
            id: "a2".to_string(),
            name: "Bash".to_string(),
            input: json!({"command": "echo second"}),
        },
    ];
    let results = exec.execute_all(&blocks);
    assert_eq!(results.len(), 2);
    assert_tool_success(&results[0]);
    assert_tool_success(&results[1]);
}
