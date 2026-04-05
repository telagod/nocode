use crate::{
    query_engine::QueryEngineModule, query_loop::QueryLoopModule,
    tool_execution::ToolExecutionModule, tool_registry::ToolRegistryModule,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewriteStatus {
    Done,
    InProgress,
    Pending,
}

impl RewriteStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::InProgress => "in_progress",
            Self::Pending => "pending",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewriteStage {
    WorkspaceBootstrap,
    QueryKernel,
    ToolExecution,
    IntegrationBridge,
    ReplReplacement,
}

impl RewriteStage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorkspaceBootstrap => "workspace-bootstrap",
            Self::QueryKernel => "query-kernel",
            Self::ToolExecution => "tool-execution",
            Self::IntegrationBridge => "integration-bridge",
            Self::ReplReplacement => "repl-replacement",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationSurface {
    pub ts_path: &'static str,
    pub rust_target: &'static str,
    pub priority: u8,
    pub module_label: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RewriteRoadmap {
    pub active_stage: RewriteStage,
    pub surfaces: &'static [MigrationSurface],
    pub checklist: &'static [RewriteChecklistItem],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RewriteChecklistItem {
    pub id: &'static str,
    pub area: &'static str,
    pub summary: &'static str,
    pub status: RewriteStatus,
    pub priority: u8,
    pub rust_target: &'static str,
    pub ts_paths: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReleaseGateStatus {
    Ready,
    InProgress,
    Blocked,
}

impl ReleaseGateStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReleaseGate {
    id: &'static str,
    summary: &'static str,
    checklist_ids: &'static [&'static str],
}

const SURFACES: [MigrationSurface; 4] = [
    MigrationSurface {
        ts_path: "src/QueryEngine.ts",
        rust_target: "nocode_core::query_engine",
        priority: 1,
        module_label: QueryEngineModule::LABEL,
    },
    MigrationSurface {
        ts_path: "src/query.ts",
        rust_target: "nocode_core::query_loop",
        priority: 1,
        module_label: QueryLoopModule::LABEL,
    },
    MigrationSurface {
        ts_path: "src/services/tools/toolExecution.ts",
        rust_target: "nocode_core::tool_execution",
        priority: 2,
        module_label: ToolExecutionModule::LABEL,
    },
    MigrationSurface {
        ts_path: "src/tools.ts",
        rust_target: "nocode_core::tool_registry",
        priority: 2,
        module_label: ToolRegistryModule::LABEL,
    },
];

const CHECKLIST: [RewriteChecklistItem; 16] = [
    RewriteChecklistItem {
        id: "kernel-roadmap",
        area: "Kernel",
        summary: "Workspace、crate 边界、迁移文档与执行清单固化",
        status: RewriteStatus::Done,
        priority: 1,
        rust_target: "nocode_core::{roadmap, README, DESIGN}",
        ts_paths: &["README.md", "FEATURES.md"],
    },
    RewriteChecklistItem {
        id: "message-response-model",
        area: "Kernel",
        summary: "QueryMessage、AssistantTurn、ModelResponse 与 transcript 响应层",
        status: RewriteStatus::InProgress,
        priority: 1,
        rust_target: "nocode_core::{message, assistant_turn, model_response, transcript}",
        ts_paths: &["src/query.ts", "src/QueryEngine.ts"],
    },
    RewriteChecklistItem {
        id: "query-loop",
        area: "Kernel",
        summary: "query loop、turn continuation、token budget、stop hooks、模型响应收敛",
        status: RewriteStatus::InProgress,
        priority: 1,
        rust_target: QueryLoopModule::LABEL,
        ts_paths: &[
            "src/query.ts",
            "src/query/tokenBudget.ts",
            "src/query/stopHooks.ts",
        ],
    },
    RewriteChecklistItem {
        id: "query-engine",
        area: "Kernel",
        summary: "session state、ask/submit、usage 统计、cache、conversation persistence",
        status: RewriteStatus::InProgress,
        priority: 1,
        rust_target: QueryEngineModule::LABEL,
        ts_paths: &[
            "src/QueryEngine.ts",
            "src/history.ts",
            "src/bootstrap/state.ts",
        ],
    },
    RewriteChecklistItem {
        id: "tooling-runtime",
        area: "Execution",
        summary: "tool registry、permission gating、tool execution runtime、hook dispatch",
        status: RewriteStatus::InProgress,
        priority: 2,
        rust_target: "nocode_core::{tool_registry, tool_execution}",
        ts_paths: &["src/tools.ts", "src/services/tools/toolExecution.ts"],
    },
    RewriteChecklistItem {
        id: "task-system",
        area: "Execution",
        summary: "LocalShellTask、LocalAgentTask、DreamTask、task coordinator、progress/update/stop APIs、drive_until_idle、shell/in-process/process agent host",
        status: RewriteStatus::InProgress,
        priority: 2,
        rust_target: "nocode runtime task system",
        ts_paths: &[
            "src/tasks.ts",
            "src/tasks/LocalShellTask/LocalShellTask.tsx",
            "src/tasks/LocalAgentTask/LocalAgentTask.tsx",
            "src/tasks/DreamTask/DreamTask.ts",
        ],
    },
    RewriteChecklistItem {
        id: "providers-config",
        area: "Providers",
        summary: "provider config、model selection、live transport plan、env auth、task budget；真实 HTTP transport、retry/backoff、SSE parser 已接通，剩 live chunk transport/capability matrix",
        status: RewriteStatus::InProgress,
        priority: 2,
        rust_target: "nocode provider layer",
        ts_paths: &["src/query/config.ts", "src/query/deps.ts", "package.json"],
    },
    RewriteChecklistItem {
        id: "bridge-remote",
        area: "Bridge",
        summary: "remote bridge、session runner、transport、pointer、permission callbacks；wire DTO、run_remote、HTTP transport、CLI remote demo 已成型",
        status: RewriteStatus::InProgress,
        priority: 3,
        rust_target: "nocode bridge subsystem",
        ts_paths: &[
            "src/bridge/bridgeMain.ts",
            "src/bridge/sessionRunner.ts",
            "src/bridge/remoteBridgeCore.ts",
            "src/bridge/replBridge.ts",
        ],
    },
    RewriteChecklistItem {
        id: "repl-ui",
        area: "UI",
        summary: "CLI entrypoint、REPL/TUI shell、`--repl`/`--tui`/`--status`/bridge demos 已可运行",
        status: RewriteStatus::InProgress,
        priority: 3,
        rust_target: "nocode CLI / TUI shell",
        ts_paths: &[
            "src/main.tsx",
            "src/screens/REPL.tsx",
            "src/components/Messages.tsx",
            "src/components/PromptInput/PromptInputQueuedCommands.tsx",
        ],
    },
    RewriteChecklistItem {
        id: "tui-session-view",
        area: "UI",
        summary: "transcript / assistant / tool / progress 已形成稳定 scrollback，仍待继续增强 renderer 与样式层",
        status: RewriteStatus::InProgress,
        priority: 3,
        rust_target: "nocode tui session view",
        ts_paths: &[
            "src/components/Messages.tsx",
            "src/screens/REPL.tsx",
            "src/utils/messages.ts",
        ],
    },
    RewriteChecklistItem {
        id: "tui-input-routing",
        area: "UI",
        summary: "补 slash command routing、输入历史、编辑态与 queued commands，而不是只停留在最小 stdin REPL",
        status: RewriteStatus::Done,
        priority: 3,
        rust_target: "nocode tui input routing",
        ts_paths: &[
            "src/components/PromptInput/PromptInputQueuedCommands.tsx",
            "src/utils/handlePromptSubmit.ts",
            "src/messageQueueManager.ts",
        ],
    },
    RewriteChecklistItem {
        id: "tui-task-panel",
        area: "UI",
        summary: "基于 TaskCoordinator / TaskDriveReport 的 task list/detail/filter/inspect 已落地，仍待更强 auto-drive 与操作面",
        status: RewriteStatus::InProgress,
        priority: 3,
        rust_target: "nocode tui task panel",
        ts_paths: &[
            "src/components/CoordinatorAgentStatus.tsx",
            "src/tasks.ts",
            "src/tasks/LocalShellTask/LocalShellTask.tsx",
        ],
    },
    RewriteChecklistItem {
        id: "tui-layout-focus",
        area: "UI",
        summary: "message/task/events pane 布局与 focus 状态机已成型，仍待 overlay/permission/runtime 深化",
        status: RewriteStatus::InProgress,
        priority: 4,
        rust_target: "nocode tui layout/focus",
        ts_paths: &[
            "src/ink/layout/engine.ts",
            "src/ink/renderer.ts",
            "src/ink.ts",
        ],
    },
    RewriteChecklistItem {
        id: "ink-runtime",
        area: "UI",
        summary: "raw terminal IO、events、resize/tick 驱动已接通；仍未达到 TS Ink runtime 等价能力",
        status: RewriteStatus::InProgress,
        priority: 4,
        rust_target: "nocode terminal renderer",
        ts_paths: &[
            "src/ink.ts",
            "src/ink/renderer.ts",
            "src/ink/layout/engine.ts",
            "src/ink/termio.ts",
        ],
    },
    RewriteChecklistItem {
        id: "plugins-voice-keybindings",
        area: "Platform",
        summary: "plugins、bundled builtins、voice mode、keybindings、onboarding migrations",
        status: RewriteStatus::Pending,
        priority: 4,
        rust_target: "nocode platform services",
        ts_paths: &[
            "src/plugins/builtinPlugins.ts",
            "src/plugins/bundled/index.ts",
            "src/voice/voiceModeEnabled.ts",
            "src/keybindings/defaultBindings.ts",
            "src/migrations/migrateAutoUpdatesToSettings.ts",
        ],
    },
    RewriteChecklistItem {
        id: "packaging-release",
        area: "Delivery",
        summary: "build pipeline、dist packaging、install flow、doctor/resume、compat migration tests",
        status: RewriteStatus::Pending,
        priority: 5,
        rust_target: "cargo workspace release pipeline",
        ts_paths: &[
            "scripts/build.ts",
            "install.sh",
            "src/screens/Doctor.tsx",
            "src/screens/ResumeConversation.tsx",
        ],
    },
];

const RELEASE_GATES: [ReleaseGate; 5] = [
    ReleaseGate {
        id: "provider-productization",
        summary: "provider live streaming、capability matrix、clear error surface",
        checklist_ids: &["providers-config"],
    },
    ReleaseGate {
        id: "task-runtime-daemon",
        summary: "task persistence/resume、cancellation、remote daemon/service",
        checklist_ids: &["task-system"],
    },
    ReleaseGate {
        id: "bridge-remote-service",
        summary: "concrete remote bridge service、resume、reconnect",
        checklist_ids: &["bridge-remote"],
    },
    ReleaseGate {
        id: "standalone-tui",
        summary: "permission/runtime/session closure without REPL fallback",
        checklist_ids: &[
            "repl-ui",
            "tui-session-view",
            "tui-task-panel",
            "tui-layout-focus",
            "ink-runtime",
        ],
    },
    ReleaseGate {
        id: "platform-release",
        summary: "doctor/install/package/CI/smoke/rollback pipeline",
        checklist_ids: &["packaging-release"],
    },
];

pub const fn default_roadmap() -> RewriteRoadmap {
    RewriteRoadmap {
        active_stage: RewriteStage::IntegrationBridge,
        surfaces: &SURFACES,
        checklist: &CHECKLIST,
    }
}

fn status_count(roadmap: &RewriteRoadmap, status: RewriteStatus) -> usize {
    roadmap
        .checklist
        .iter()
        .filter(|item| item.status == status)
        .count()
}

fn total_count(roadmap: &RewriteRoadmap) -> usize {
    roadmap.checklist.len()
}

fn completion_percent(roadmap: &RewriteRoadmap) -> usize {
    let total = total_count(roadmap);
    if total == 0 {
        return 0;
    }
    (status_count(roadmap, RewriteStatus::Done) * 100) / total
}

fn checklist_item_by_id<'a>(
    roadmap: &'a RewriteRoadmap,
    id: &str,
) -> Option<&'a RewriteChecklistItem> {
    roadmap.checklist.iter().find(|item| item.id == id)
}

fn release_gate_status(roadmap: &RewriteRoadmap, gate: &ReleaseGate) -> ReleaseGateStatus {
    let statuses = gate
        .checklist_ids
        .iter()
        .filter_map(|id| checklist_item_by_id(roadmap, id))
        .map(|item| item.status)
        .collect::<Vec<_>>();
    if !statuses.is_empty() && statuses.iter().all(|status| *status == RewriteStatus::Done) {
        return ReleaseGateStatus::Ready;
    }
    if statuses.contains(&RewriteStatus::InProgress) {
        return ReleaseGateStatus::InProgress;
    }
    ReleaseGateStatus::Blocked
}

fn release_gate_count(roadmap: &RewriteRoadmap, status: ReleaseGateStatus) -> usize {
    RELEASE_GATES
        .iter()
        .filter(|gate| release_gate_status(roadmap, gate) == status)
        .count()
}

fn launch_readiness(roadmap: &RewriteRoadmap) -> &'static str {
    if release_gate_count(roadmap, ReleaseGateStatus::Ready) == RELEASE_GATES.len() {
        "yes"
    } else {
        "no"
    }
}

fn render_checklist_group(
    output: &mut String,
    roadmap: &RewriteRoadmap,
    heading: &str,
    status: RewriteStatus,
    limit: usize,
) {
    output.push_str(&format!("{heading}:\n"));
    let mut emitted = 0usize;
    for item in roadmap
        .checklist
        .iter()
        .filter(|item| item.status == status)
    {
        if emitted >= limit {
            break;
        }
        output.push_str(&format!(
            "- P{} [{}] {} :: {}\n",
            item.priority, item.area, item.id, item.summary
        ));
        emitted += 1;
    }
    if emitted == 0 {
        output.push_str("- none\n");
    }
}

pub fn render_status(roadmap: &RewriteRoadmap) -> String {
    let mut output = format!("active-stage: {}\n", roadmap.active_stage.as_str());
    output.push_str(&format!(
        "launch-readiness: {} (ready={} in_progress={} blocked={})\n",
        launch_readiness(roadmap),
        release_gate_count(roadmap, ReleaseGateStatus::Ready),
        release_gate_count(roadmap, ReleaseGateStatus::InProgress),
        release_gate_count(roadmap, ReleaseGateStatus::Blocked)
    ));
    output.push_str(&format!(
        "summary: done={} in_progress={} pending={}\n",
        status_count(roadmap, RewriteStatus::Done),
        status_count(roadmap, RewriteStatus::InProgress),
        status_count(roadmap, RewriteStatus::Pending)
    ));
    output.push_str(&format!(
        "progress: completed={}/{} ({}%) active={} backlog={}\n",
        status_count(roadmap, RewriteStatus::Done),
        total_count(roadmap),
        completion_percent(roadmap),
        status_count(roadmap, RewriteStatus::InProgress),
        status_count(roadmap, RewriteStatus::Pending)
    ));

    render_checklist_group(
        &mut output,
        roadmap,
        "current-focus",
        RewriteStatus::InProgress,
        4,
    );
    render_checklist_group(
        &mut output,
        roadmap,
        "next-blockers",
        RewriteStatus::Pending,
        4,
    );

    output.push_str("release-gates:\n");
    for gate in &RELEASE_GATES {
        output.push_str(&format!(
            "- [{}] {} :: {}\n",
            release_gate_status(roadmap, gate).as_str(),
            gate.id,
            gate.summary
        ));
    }

    output.push_str("migration-surfaces:\n");
    for surface in roadmap.surfaces {
        let line = format!(
            "- P{} [{}] {} -> {}\n",
            surface.priority, surface.module_label, surface.ts_path, surface.rust_target
        );
        output.push_str(&line);
    }

    output.push_str("release-blockers:\n");
    output.push_str("- provider live streaming + capability/error matrix\n");
    output.push_str("- task persistence/resume + remote daemon/service\n");
    output.push_str("- concrete remote bridge service + reconnect/resume\n");
    output.push_str("- standalone TUI permission/runtime/session closure\n");
    output.push_str("- packaging/CI/compat/release pipeline\n");

    output.push_str("rewrite-checklist:\n");
    for item in roadmap.checklist {
        let ts_paths = item.ts_paths.join(", ");
        let line = format!(
            "- [{}] P{} {} :: {} -> {} | ts={}\n",
            item.status.as_str(),
            item.priority,
            item.area,
            item.summary,
            item.rust_target,
            ts_paths
        );
        output.push_str(&line);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::{RewriteStage, RewriteStatus, default_roadmap, render_status};

    #[test]
    fn roadmap_tracks_current_active_stage() {
        let roadmap = default_roadmap();
        assert_eq!(roadmap.active_stage, RewriteStage::IntegrationBridge);
    }

    #[test]
    fn status_contains_query_targets() {
        let rendered = render_status(&default_roadmap());
        assert!(rendered.contains("src/QueryEngine.ts"));
        assert!(rendered.contains("src/query.ts"));
        assert!(rendered.contains("migration-surfaces:"));
        assert!(rendered.contains("launch-readiness: no"));
        assert!(rendered.contains("summary: done="));
        assert!(rendered.contains("progress: completed="));
        assert!(rendered.contains("current-focus:"));
        assert!(rendered.contains("next-blockers:"));
        assert!(rendered.contains("release-gates:"));
        assert!(rendered.contains("rewrite-checklist:"));
        assert!(rendered.contains("task budget"));
    }

    #[test]
    fn roadmap_contains_pending_and_in_progress_tracks() {
        let roadmap = default_roadmap();
        assert!(
            roadmap
                .checklist
                .iter()
                .any(|item| item.status == RewriteStatus::InProgress)
        );
        assert!(
            roadmap
                .checklist
                .iter()
                .any(|item| item.status == RewriteStatus::Pending)
        );
    }
}
