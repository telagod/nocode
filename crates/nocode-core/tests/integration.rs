use nocode_core::memory_store::{MemoryEntry, MemoryStore, MemoryType};
use nocode_core::mock_service::ParityTestRunner;

// ---------------------------------------------------------------------------
// MockAnthropicService parity scenarios
// ---------------------------------------------------------------------------

#[test]
fn streaming_text_scenario() {
    let mut runner = ParityTestRunner::new();
    let result = runner.run_scenario("streaming_text");
    assert!(result.is_ok());
    let result = result.unwrap();
    assert!(result.response_matched);
    assert_eq!(result.request_count, 1);
}

#[test]
fn read_file_roundtrip_scenario() {
    let mut runner = ParityTestRunner::new();
    let result = runner.run_scenario("read_file_roundtrip").unwrap();
    runner.assert_scenario_passed(&result);
    // Should contain a tool_use block for Read
    assert!(result.captured[0].body.contains("read_file"));
}

#[test]
fn multi_tool_turn_scenario() {
    let mut runner = ParityTestRunner::new();
    let result = runner.run_scenario("multi_tool_turn").unwrap();
    runner.assert_scenario_passed(&result);
    assert_eq!(result.request_count, 1);
}

#[test]
fn bash_stdout_roundtrip_scenario() {
    let mut runner = ParityTestRunner::new();
    let result = runner.run_scenario("bash_stdout_roundtrip").unwrap();
    runner.assert_scenario_passed(&result);
}

#[test]
fn token_cost_reporting_scenario() {
    let mut runner = ParityTestRunner::new();
    let result = runner.run_scenario("token_cost_reporting").unwrap();
    runner.assert_scenario_passed(&result);
}
#[test]
fn write_file_denied_scenario() {
    let mut runner = ParityTestRunner::new();
    let result = runner.run_scenario("write_file_denied").unwrap();
    runner.assert_scenario_passed(&result);
}

#[test]
fn nonexistent_scenario_returns_error() {
    let mut runner = ParityTestRunner::new();
    let result = runner.run_scenario("does_not_exist");
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Memory store integration: save / search / delete roundtrip
// ---------------------------------------------------------------------------

#[test]
fn memory_save_and_search_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(tmp.path().to_str().unwrap());

    // Save
    let entry = MemoryEntry {
        name: "integration-test".to_string(),
        description: "roundtrip test entry".to_string(),
        memory_type: MemoryType::Feedback,
        content: "Use real database for tests.".to_string(),
        file_name: "integration_test.md".to_string(),
    };
    store.save(&entry).unwrap();
    store.add_to_index(&entry).unwrap();

    // Search — should find it
    let found = store.search("real database").unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "integration-test");
    assert_eq!(found[0].memory_type, MemoryType::Feedback);

    // List — should include it
    let all = store.list().unwrap();
    assert_eq!(all.len(), 1);

    // Index — should have entry
    let idx = store.load_index().unwrap();
    assert_eq!(idx.entries.len(), 1);
    assert_eq!(idx.entries[0].file_name, "integration_test.md");

    // Delete
    store.delete("integration_test.md").unwrap();
    store.remove_from_index("integration_test.md").unwrap();

    // Search — should NOT find it
    let after_delete = store.search("real database").unwrap();
    assert!(after_delete.is_empty());

    // Index — should be empty
    let idx_after = store.load_index().unwrap();
    assert!(idx_after.entries.is_empty());
}

#[test]
fn memory_store_multiple_types_and_filter() {
    let tmp = tempfile::tempdir().unwrap();
    let store = MemoryStore::new(tmp.path().to_str().unwrap());

    let entries = vec![
        MemoryEntry {
            name: "user-role".to_string(),
            description: "user is a pentester".to_string(),
            memory_type: MemoryType::User,
            content: "Senior pentester.".to_string(),
            file_name: "user_role.md".to_string(),
        },
        MemoryEntry {
            name: "proj-goal".to_string(),
            description: "ship v1".to_string(),
            memory_type: MemoryType::Project,
            content: "Ship v1 by Friday.".to_string(),
            file_name: "proj_goal.md".to_string(),
        },
        MemoryEntry {
            name: "ref-linear".to_string(),
            description: "linear project link".to_string(),
            memory_type: MemoryType::Reference,
            content: "Bugs tracked in Linear INGEST.".to_string(),
            file_name: "ref_linear.md".to_string(),
        },
    ];

    for e in &entries {
        store.save(e).unwrap();
    }

    let all = store.list().unwrap();
    assert_eq!(all.len(), 3);

    // Search by content keyword
    let linear = store.search("Linear").unwrap();
    assert_eq!(linear.len(), 1);
    assert_eq!(linear[0].name, "ref-linear");

    // find_by_name
    let found = store.find_by_name("proj-goal").unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().content, "Ship v1 by Friday.");

    let missing = store.find_by_name("nonexistent").unwrap();
    assert!(missing.is_none());
}
