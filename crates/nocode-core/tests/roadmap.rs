use nocode_core::{
    QueryEngineModule, QueryLoopModule, QuerySource, TaskBudget, ToolExecutionModule,
    ToolRegistryModule, default_roadmap, render_status,
};

#[test]
fn roadmap_lists_all_seed_surfaces() {
    let roadmap = default_roadmap();
    assert_eq!(roadmap.surfaces.len(), 4);
}

#[test]
fn status_output_mentions_tool_execution() {
    let rendered = render_status(&default_roadmap());
    assert!(rendered.contains("launch-readiness: no"));
    assert!(rendered.contains("release-gates:"));
    assert!(rendered.contains("[in_progress] provider-productization"));
    assert!(rendered.contains("[blocked] platform-release"));
    assert!(rendered.contains("tool_execution"));
    assert!(rendered.contains("[tool-execution]"));
    assert!(rendered.contains("progress: completed="));
    assert!(rendered.contains("current-focus:"));
    assert!(rendered.contains("next-blockers:"));
}

#[test]
fn module_constants_match_expected_ts_sources() {
    assert_eq!(QueryEngineModule::TS_SOURCE, "src/QueryEngine.ts");
    assert_eq!(QueryLoopModule::TS_SOURCE, "src/query.ts");
    assert_eq!(
        ToolExecutionModule::TS_SOURCE,
        "src/services/tools/toolExecution.ts"
    );
    assert_eq!(ToolRegistryModule::TS_SOURCE, "src/tools.ts");
}

#[test]
fn task_budget_and_query_source_are_stable_seed_types() {
    let budget = TaskBudget { total: 10_000 };
    assert_eq!(budget.total, 10_000);
    assert_eq!(QuerySource::Sdk.as_str(), "sdk");
}
