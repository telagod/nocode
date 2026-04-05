# nocode DESIGN

## 设计目标

- 用 Rust 重建 `redcode` 的执行内核，产物名固定为 `nocode`。
- 先完成 query/provider/task/bridge/TUI 的独立主链，再谈 TS 版替代。
- 从第一天保持可编译、可测试、可观察、可继续拆分。
- 用显式 checklist 管住迁移范围，避免把 TS 全量 UI 与 feature flags 一口吞下。

## 非目标

- 一期不追求与 TS Ink UI 完全像素级对齐。
- 一期不保留 embedded proxy、`/free`、telemetry 改造线。
- 一期不迁 plugin marketplace、voice、bundled builtins 全家桶。
- 一期不承诺跨机 bridge/daemon 生产可用。

## 为什么不是直接重写完整 redcode

TS 侧 `main.tsx`、`REPL.tsx`、Ink renderer、bridge、feature flags、platform service 耦合极深。

若直接照搬：

- 迁移面会被 `/free`、flags、plugins、voice 等支线拖死
- CLI 行为会继续绑死在 Bun/Ink 的历史包袱上
- 很难尽快做出一个可独立运行、可持续演进的 Rust 主线

所以 `nocode` 的路线是：

1. 先重建内核
2. 再补最小独立交互壳
3. 最后决定哪些 `redcode` 能力值得迁、哪些应该直接砍

## 当前架构

```text
nocode (CLI/TUI shell)
  -> repl.rs / tui.rs / task_panel.rs / main.rs
  -> nocode-core
       -> query_engine / query_loop / message / transcript
       -> provider / provider_transport / query_deps
       -> tool_registry / tool_execution
       -> task_runtime
       -> bridge_runtime
       -> persistence / resume / roadmap
```

## 命名决策

会话级 structured output 已统一抬升为 canonical result term：

- 对外展示名：`response-result`
- Rust / wire 字段名：`response_result`
- task 面板上的聚合展示：`result`

保留旧名 `structured_output` 的边界只有两处：

1. provider JSON schema request name 与 `structured_output_failure` 这类外部/错误契约
2. bridge / process-agent wire 的 backward-compatible decode alias

设计约束：

- 新增 session/task/bridge/status/diagnostics 结果面时，不再引入第二套命名
- 任何新 wire 输出都必须首选 `response_result`
- 若保留 `structured_output`，必须能说明它属于 provider 契约或兼容层

## 与 redcode 的详细对照

| 维度 | `redcode` 基线 | `nocode` 现状 | 判定 |
| --- | --- | --- | --- |
| Query kernel | TS `src/QueryEngine.ts` + `src/query.ts` 完整主链 | Rust `query_engine` / `query_loop` 已可独立跑 | 基本通，但仍需继续细化 state/continuation |
| Provider | Anthropic 主路，且项目层叠加 `/free` 代理变体 | Claude/OpenAI typed adapter + HTTP transport | 可用但未产品化 |
| Tool runtime | TS 工具、权限、hooks 与 UI 深耦合 | `tool_registry` / `tool_execution` 已独立 | 主链在，但权限体验仍浅 |
| Task runtime | TS task UI/生命周期/后台链更多 | Rust 已有 shell/agent/dream/process host | 仍缺持久化、remote daemon、审计 |
| Bridge | TS 有更深远控和会话体系 | Rust 有 runner + HTTP transport demo | 仍不是完整 bridge service |
| TUI | Ink REPL、dialogs、permission 浮层、输入体验完整 | standalone TUI 四窗格已可操作 | 仍非替代级 |
| Release | `install.sh`、build variants、flags、Doctor/Resume 等 | Cargo workspace | 差距最大 |
| `/free` / proxy | `redcode` 的一条产品主线 | `nocode` 明确不迁 | 有意差异，不是缺陷 |

## 已落地能力

### Kernel / provider

- `QueryMessage`、`AssistantTurn`、`ModelResponse`、`TranscriptEntry` 已把会话与响应边界类型化。
- `QueryEngine` + `QueryLoop` 已可完成 submit、tool batch flush、stop hook、budget、assistant completion、persistence dispatch。
- provider adapter 已覆盖 `Claude Messages`、`OpenAI Chat Completions`、`OpenAI Responses`。
- provider transport 已具备真实 HTTP、retry/backoff、SSE body parser。

### Task runtime

- `TaskCoordinator` 已具备 queue/list/inspect/stop/drive API。
- `LiveTaskShellHost`、`InProcessAgentHost`、`ProcessTaskAgentHost`、`DefaultDreamHost` 已接上。
- `ProcessTaskAgentHost` 已有 daemon supervisor、restart budget、failure-kind backoff profile、runtime observability。

### Bridge / session

- `SessionRunner` 已有 local 与 remote 路径。
- `RemoteBridgeTransport` trait、wire DTO、permission callback 回传已成型。
- `HttpRemoteBridgeTransport` 已可演示真实 HTTP 往返。
- `resume_with_reader()` 已可从本地 JSONL 恢复 session snapshot。
- bridge summary / submitted wire / diagnostics 结果面已统一到 `response-result` / `response_result`。

### CLI / TUI

- `--status`、`--repl`、`--tui`、`--bridge-once`、`--bridge-remote-once` 可运行。
- TUI 已有 transcript/task list/task detail/events 四窗格。
- pane focus、scroll、task inspect/filter、slash routing、输入历史、draft/queue 已落地。
- `/tasks result:agent`、`/tasks result-structured:yes|no` 已可直接过滤结果类型与结构化结果任务。

## 长 TODO

### P1. Provider 产品化

目标：把“能请求”推进到“能稳定上线”。

- [ ] live chunk streaming transport，而不只是完整 SSE body parser。
- [ ] stream event state machine：delta、tool call、assistant turn finish、abort、partial flush。
- [ ] capability matrix：显式标记 stream、tool use、json schema、reasoning、attachments。
- [ ] 更细错误面：auth、quota、rate limit、timeout、transport、decode、protocol mismatch。
- [ ] model selection 策略继续细化，避免 provider/model 能力错配。
- [ ] provider integration tests：Claude/OpenAI 各 request shape、error mapping、streaming。
- [ ] 更稳 transport seam，为 future async/client swap 留口。

Definition of done:

- CLI/TUI 能看到稳定 streaming 输出。
- provider 错误能稳定归类，不再是大段字符串错误。
- 各 provider/model 能力能被程序显式判断，而不是调用后碰运气。

### P2. Task runtime / daemon

目标：把“本地可跑任务”推进到“可恢复、可审计、可守护”。

- [ ] 跨 session 持久任务表。
- [ ] 任务恢复、重连、resume 后的状态修复。
- [ ] 正式 external agent host / daemon host 协议，而不是只停留在最小 stdin/stdout JSON。
- [ ] concrete remote daemon transport。
- [ ] cancellation / cleanup / kill escalation / timeout policy。
- [ ] task 审计链：spawn、permission、retry、restart、kill、final status。
- [ ] shell/agent/dream 的统一事件流与 TUI 展示模型。
- [ ] 更完整 supervisor 策略对象：restart/backoff/classification/history persistence。

Definition of done:

- 任务退出后可恢复查看。
- daemon/host 异常重启不丢上下文。
- stop/kill/timeout 路径都有清晰可见的最终状态。

### P3. Bridge / session

目标：把“trait + HTTP demo”推进到“真正远端可用”。

- [ ] concrete bridge service。
- [ ] session registry、remote session pointer、session resume。
- [ ] 长连接 transport：WebSocket 或等价方案。
- [ ] request/response/permission callback 版本化。
- [ ] reconnect、heartbeat、timeout、auth refresh。
- [ ] remote error mapping：网络错、权限错、序列化错、session 丢失。
- [ ] bridge integration tests：local loopback、real HTTP、断线恢复。

Definition of done:

- `--bridge-remote-once` 不再只是 demo 入口，而是可对真实服务稳定互通。
- session 中断后能恢复，而不是直接丢失。

### P4. Standalone TUI

目标：把“最小可操作面板”推进到“可独立承载主链”。

- [ ] permission prompt / overlay / modal。
- [ ] live session refresh 与后台 runtime 推送整合。
- [ ] transcript renderer：assistant/tool/progress/error 明确分型。
- [ ] input editor：selection、更多 keybindings、编辑态反馈。
- [ ] queued origin / local origin / remote origin 呈现统一。
- [ ] error panel / diagnostics panel / richer footer。
- [ ] task panel 深化：操作键、批量动作、自动刷新、失败重试。
- [ ] bridge/session/runtime 常驻状态可视化。

Definition of done:

- 不依赖 REPL fallback，也能在 TUI 内完成 query、permission、task、bridge 四条主链。
- UI 能清楚区分消息、工具、任务、错误、系统事件。

### P5. Platform / release

目标：把“代码工程”推进到“可交付产品”。

- [ ] doctor / compat / resume UX。
- [ ] 打包产物、安装方式、发布流水线。
- [ ] CI matrix：fmt、clippy、test、smoke、integration。
- [ ] 远端 bridge integration test。
- [ ] 安装后 smoke path：provider、REPL、TUI、bridge、task。
- [ ] 配置迁移与回滚策略。
- [ ] 灰度切换标准：何时从 `redcode` 切到 `nocode`。

Definition of done:

- 新机器可以稳定安装并通过 smoke。
- 发布失败有可执行回滚路径。

### P6. redcode 差异化决策

目标：明确哪些东西不迁，避免无限追 parity。

- [ ] `/free`、embedded proxy 路线永久不迁，并在文档中持续声明。
- [ ] feature flags 审计：哪些功能是产品能力，哪些只是 TS 历史噪音。
- [ ] plugins / bundled builtins 的迁移策略。
- [ ] voice / onboarding / keybindings 中哪些值得保留。
- [ ] 与 `redcode` 的兼容边界：命令、配置、会话、插件、脚本。

Definition of done:

- 迁移范围稳定，不再被 TS 历史功能拖着跑。

## 当前执行顺序

### 第一优先级

- P1 Provider 产品化
- P4 Standalone TUI
- P5 基础 release/CI 骨架

原因：这三层决定 `nocode` 能不能先形成“可长期内部试运行”的版本。

### 第二优先级

- P2 Task runtime / daemon
- P3 Bridge / session

原因：这两层决定 `nocode` 能不能脱离本地 demo，进入真正远端/持续运行场景。

### 第三优先级

- P6 redcode 差异化决策收口

原因：这是长期边界治理，不是阻塞主链运行的第一刀。

## 下一阶段建议

若按“逐步推进”而不是大而化之，建议接下来三刀依次是：

1. provider capability matrix + stream mode 观测面
2. TUI permission / diagnostics 面板骨架
3. roadmap / status 输出改成更清楚的 blocker + progress 摘要

## 上线判据

如果目标是“替代 redcode 上线”，至少同时满足：

1. provider 有 live streaming、能力矩阵、清晰错误面。
2. task 有持久化、恢复、取消、daemon/service。
3. bridge 有真正远端服务、resume、重连。
4. TUI 能独立完成主链交互。
5. release / CI / compat / rollback 成套。

未满足以上五条前，`nocode` 的定位都应维持为 internal preview。

## 变更记录

### 2026-04-04

- 文档按“README 运行手册 / DESIGN 迁移战图”重新分层。
- 补上与 `redcode` 的逐层对照，而不再只写抽象 TODO。
- 将剩余工作重排为 P1-P6 长 TODO，并给出 Definition of done 与执行顺序。
- 会话级结果命名统一到 `response-result` / `response_result`，旧 `structured_output` 收缩到 provider 契约与兼容层。
