# nocode vs redcode 功能对齐清单

> 生成时间：2026-04-05
> 基准：redcode TS/Bun (507,396 LOC, 1,910 files) vs nocode Rust (~22,000 LOC, 43 files)

## 图例

- ✅ 已实现且可用
- 🔶 部分实现 / 骨架存在
- ❌ 未实现
- 🚫 明确不迁（设计决策）

---

## 1. Provider / Model 层

| 能力 | redcode | nocode | 差距 |
|------|---------|--------|------|
| Claude Messages API | ✅ @anthropic-ai/sdk | ✅ 原生 HTTP | — |
| OpenAI Chat Completions | ✅ | ✅ | — |
| OpenAI Responses | ✅ | ✅ | — |
| AWS Bedrock | ✅ Bedrock SDK | ❌ | 缺整个 adapter |
| Google Vertex AI | ✅ Vertex SDK | ❌ | 缺整个 adapter |
| Anthropic Foundry | ✅ Foundry SDK | ❌ | 缺整个 adapter |
| Mock provider | ✅ | ✅ | — |
| Live SSE streaming | ✅ | 🔶 管道已通，TUI 异步渲染已搭 | 缺真实 API 端到端验证 |
| Prompt caching | ✅ cache control | ❌ | — |
| Structured output / JSON schema | ✅ | 🔶 schema 字段存在，未产品化 | — |
| Cost tracking / token billing | ✅ | 🔶 UsageTracker 存在 | 缺 USD 计费 |
| Reasoning effort (OpenAI) | ✅ | ✅ env var 支持 | — |
| Thinking mode | ✅ extended thinking | 🔶 Adaptive/Disabled enum | 缺 thinkback 播放 |
| Model selection / fallback | ✅ | ✅ | — |
| Retry / backoff | ✅ | ✅ 指数退避 | — |
| `/free` embedded proxy | ✅ cursor2api | 🚫 明确不迁 | 设计决策 |

---

## 2. Tool 层

| 工具 | redcode | nocode | 差距 |
|------|---------|--------|------|
| Read (FileRead) | ✅ | ✅ | — |
| Edit (FileEdit) | ✅ | ✅ | — |
| Write (FileWrite) | ✅ 434 LOC | ✅ | — |
| Bash | ✅ 2,621 LOC permissions | ✅ 基础实现 | 缺权限沙箱、只读验证 |
| Glob | ✅ | ✅ 基于 find | — |
| Grep | ✅ 577 LOC | ✅ 基于 grep -rn | — |
| WebSearch | ✅ | ❌ | — |
| WebFetch | ✅ 530 LOC | ❌ | — |
| Agent (spawn sub-agent) | ✅ | ❌ 注册但无实现 | — |
| NotebookEdit | ✅ 490 LOC | ❌ | — |
| PowerShell | ✅ 2,049 LOC | ❌ | — |
| LSP Tool | ✅ 860 LOC | ❌ | — |
| MCP Tool | ✅ | ❌ | — |
| ListMcpResources | ✅ | ❌ | — |
| ReadMcpResource | ✅ | ❌ | — |
| McpAuth | ✅ 215 LOC | ❌ | — |
| SkillTool | ✅ | ❌ | — |
| TaskCreate/Get/List/Update/Stop/Output | ✅ 6 tools | ❌ | — |
| TeamCreate/Delete | ✅ | ❌ | — |
| SendMessage | ✅ | ❌ | — |
| AskUserQuestion | ✅ | ❌ | — |
| TodoWrite | ✅ | ❌ | — |
| EnterPlanMode / ExitPlanMode | ✅ 493 LOC | ❌ | — |
| EnterWorktree / ExitWorktree | ✅ | ❌ | — |
| ConfigTool | ✅ 467 LOC | ❌ | — |
| ToolSearch | ✅ | ❌ | — |
| BriefTool | ✅ | ❌ | — |
| SleepTool | ✅ | ❌ | — |
| ScheduleCron / RemoteTrigger | ✅ feature-gated | ❌ | — |
| SyntheticOutput | ✅ | ❌ | — |
| **合计** | **42 tools** | **6 tools** | **缺 36 tools** |

---

## 3. Command 层 (Slash Commands)

| 命令类别 | redcode | nocode | 差距 |
|----------|---------|--------|------|
| 基础 (/help, /quit, /status) | ✅ | ✅ | — |
| 历史 (/history, /inputs) | ✅ | ✅ | — |
| 任务管理 (/tasks, /task-*) | ✅ | ✅ 完整 | — |
| 队列 (/queue, /queue-slash) | ✅ | ✅ | — |
| 草稿 (/draft, /edit, /append, /send) | ✅ | ✅ | — |
| 焦点 (/focus) | ✅ | ✅ | — |
| /model | ✅ | ❌ | — |
| /config | ✅ | ❌ | — |
| /permissions | ✅ | ❌ | — |
| /commit, /commit-push-pr | ✅ | ❌ | — |
| /diff | ✅ | ❌ | — |
| /review, /ultrareview | ✅ | ❌ | — |
| /plan, /ultraplan | ✅ | ❌ | — |
| /compact | ✅ | ❌ | — |
| /memory | ✅ | ❌ | — |
| /agents | ✅ | ❌ | — |
| /skills | ✅ | ❌ | — |
| /plugin | ✅ | ❌ | — |
| /mcp | ✅ | ❌ | — |
| /bridge, /remote-control | ✅ | 🔶 --bridge-once 存在 | — |
| /ide | ✅ | ❌ | — |
| /desktop, /mobile | ✅ | ❌ | — |
| /voice | ✅ | ❌ | — |
| /vim | ✅ | ❌ | — |
| /theme | ✅ | ❌ | — |
| /cost, /usage | ✅ | ❌ | — |
| /insights | ✅ 3,200 LOC | ❌ | — |
| /doctor | ✅ | ❌ | — |
| /login, /logout | ✅ | ❌ | — |
| /free | ✅ | 🚫 不迁 | — |
| /thinkback, /ultrathink | ✅ | ❌ | — |
| /resume, /rewind | ✅ | ❌ | — |
| /teleport | ✅ | ❌ | — |
| /export, /copy | ✅ | ❌ | — |
| /env | ✅ | ❌ | — |
| /keybindings | ✅ | ❌ | — |
| /bughunter, /security-review | ✅ | ❌ | — |
| /passes, /feedback | ✅ | ❌ | — |
| **合计** | **~103 commands** | **~30 commands** | **缺 ~73 commands** |

<!-- ALIGNMENT_CONTINUE -->

## 4. UI / 交互层

| 能力 | redcode | nocode | 差距 |
|------|---------|--------|------|
| Ink/React 富文本 REPL | ✅ 349 TSX, 77K LOC | ❌ 纯文本 REPL | 完全不同的渲染架构 |
| TUI 多窗格 | ❌ (Ink 单流) | ✅ 四窗格 crossterm | nocode 独有 |
| 消息类型渲染 (38 种) | ✅ | 🔶 文本级别 role badge | 缺富文本 |
| Permission 对话框 (44 组件) | ✅ 完整 UI | 🔶 F3 overlay 骨架 | 缺真实 lifecycle |
| 虚拟滚动 | ✅ VirtualMessageList | ❌ | — |
| 代码高亮 | ✅ HighlightedCode | ❌ | — |
| Diff 视图 | ✅ DiffDialog/Detail/FileList | ❌ | — |
| 搜索 (GlobalSearch, QuickOpen) | ✅ | ❌ | — |
| 主题系统 | ✅ ThemeProvider | ❌ | — |
| Vim 模式 | ✅ | ❌ | — |
| 输入历史 | ✅ | ✅ | — |
| 草稿/队列 | ✅ | ✅ | — |
| Spinner / 进度条 | ✅ | ❌ | — |
| 图片消息 | ✅ UserImageMessage | ❌ | — |
| 剪贴板集成 | ✅ useCopyOnSelect | ❌ | — |
| 桌面应用 handoff | ✅ DesktopHandoff | ❌ | — |
| Onboarding 引导 | ✅ Onboarding.tsx | ❌ | — |
| 自动更新 | ✅ NativeAutoUpdater | ❌ | — |

---

## 5. Task / Agent 层

| 能力 | redcode | nocode | 差距 |
|------|---------|--------|------|
| Shell task | ✅ | ✅ | — |
| Agent task (in-process) | ✅ | ✅ InProcessAgentHost | — |
| Agent task (process) | ✅ | ✅ ProcessTaskAgentHost | — |
| Dream task | ✅ | ✅ | — |
| Daemon supervisor | ✅ | ✅ restart/backoff/failure 完整 | — |
| Task 持久化 (跨 session) | ✅ | ❌ | — |
| Task resume / 恢复 | ✅ | ❌ | — |
| Task 审计链 | ✅ | ❌ | — |
| Remote daemon | ✅ | ❌ | — |
| Agent swarm (多 agent 协同) | ✅ 21 files swarm/ | ❌ | — |
| Team create/delete | ✅ | ❌ | — |
| Inter-agent messaging | ✅ SendMessage | ❌ | — |
| Agent creation wizard | ✅ 21 wizard steps | ❌ | — |
| Scheduled tasks / Cron | ✅ feature-gated | ❌ | — |
| Background task dialog | ✅ 651 LOC | 🔶 TUI task panel | — |

---

## 6. Bridge / Session 层

| 能力 | redcode | nocode | 差距 |
|------|---------|--------|------|
| Local bridge | ✅ | ✅ | — |
| Remote bridge (HTTP) | ✅ | 🔶 demo transport | 缺真正 service |
| Session registry | ✅ | ❌ | — |
| Session resume | ✅ | 🔶 resume_with_reader 存在 | 缺产品化 |
| WebSocket 长连接 | ✅ | ❌ | — |
| Reconnect / heartbeat | ✅ | ❌ | — |
| Permission callback 回传 | ✅ | ✅ wire 已通 | — |
| Bridge event streaming | ✅ | ✅ BridgeEventWire | — |
| SSH session | ✅ useSSHSession | ❌ | — |
| Direct connect | ✅ useDirectConnect | ❌ | — |

---

## 7. Persistence / 存储层

| 能力 | redcode | nocode | 差距 |
|------|---------|--------|------|
| Session persistence | ✅ | ✅ JSONL | — |
| Transcript persistence | ✅ | ✅ | — |
| History store | ✅ | ✅ | — |
| File history | ✅ | ✅ | — |
| Resume snapshot | ✅ | ✅ | — |
| Read file cache | ✅ | 🔶 字段存在 | — |
| Secure storage | ✅ 6 files | ❌ | — |
| Settings sync | ✅ | ❌ | — |

---

## 8. Permission / 安全层

| 能力 | redcode | nocode | 差距 |
|------|---------|--------|------|
| Tool permission gating | ✅ 24 files | 🔶 ToolPermissionContext 存在 | 缺规则引擎 |
| Bash 权限沙箱 | ✅ 2,621 LOC | ❌ | — |
| Bash 只读验证 | ✅ 1,990 LOC | ❌ | — |
| PowerShell 路径验证 | ✅ 2,049 LOC | ❌ | — |
| Permission rules UI | ✅ 1,178 LOC | ❌ | — |
| Auto-approve / deny | ✅ | ✅ bridge 层 | — |
| Classifier approvals | ✅ | ❌ | — |
| TUI permission overlay | ❌ | ✅ F3 approve/deny | nocode 独有 |

---

## 9. MCP (Model Context Protocol)

| 能力 | redcode | nocode | 差距 |
|------|---------|--------|------|
| MCP client | ✅ | ❌ | — |
| MCP auth (OAuth) | ✅ 2,465 LOC | ❌ | — |
| MCP tool execution | ✅ | ❌ | — |
| MCP resource listing/reading | ✅ | ❌ | — |
| MCP server management | ✅ 12 components | ❌ | — |
| MCP elicitation | ✅ 1,168 LOC | ❌ | — |
| In-process transport | ✅ | ❌ | — |
| SDK control transport | ✅ | ❌ | — |
| VS Code MCP | ✅ | ❌ | — |
| **整个 MCP 子系统** | **✅ ~20 files** | **❌ 零实现** | **完全缺失** |

---

## 10. Plugin / Skill 系统

| 能力 | redcode | nocode | 差距 |
|------|---------|--------|------|
| Plugin 安装/管理 | ✅ 44 files | ❌ | — |
| Skill 系统 | ✅ | ❌ | — |
| Plugin CLI commands | ✅ | ❌ | — |
| **整个 Plugin 子系统** | **✅** | **❌ 零实现** | **完全缺失** |

---

## 11. IDE / 外部集成

| 能力 | redcode | nocode | 差距 |
|------|---------|--------|------|
| VS Code 集成 | ✅ | ❌ | — |
| JetBrains 集成 | ✅ | ❌ | — |
| IDE logging | ✅ | ❌ | — |
| IDE selection | ✅ | ❌ | — |
| IDE @ mentions | ✅ | ❌ | — |
| Desktop app | ✅ | ❌ | — |
| Mobile | ✅ | ❌ | — |
| Deep linking | ✅ 6 files | ❌ | — |

---

## 12. Auth / 账户

| 能力 | redcode | nocode | 差距 |
|------|---------|--------|------|
| OAuth client | ✅ | ❌ | — |
| Login / logout | ✅ | ❌ | — |
| API key verification | ✅ | ❌ (直接用 env var) | — |
| Session ingress auth | ✅ | ❌ | — |
| Referral / passes | ✅ | ❌ | — |
| Credit grants | ✅ | ❌ | — |

---

## 13. Analytics / Telemetry

| 能力 | redcode | nocode | 差距 |
|------|---------|--------|------|
| GrowthBook feature flags | ✅ 88 flags | ❌ | — |
| Datadog integration | ✅ | ❌ | — |
| Event logging | ✅ | ❌ | — |
| Metrics opt-out | ✅ | ❌ | — |
| FPS metrics | ✅ | ❌ | — |
| **整个 Telemetry 子系统** | **✅ ~9 files** | **❌ 零实现** | **完全缺失** |

---

## 14. Context / Memory

| 能力 | redcode | nocode | 差距 |
|------|---------|--------|------|
| Context compaction | ✅ 15 files | 🔶 Compactor trait 存在 | 缺真实实现 |
| Session memory | ✅ 3 files | ❌ | — |
| Memory extraction | ✅ 2 files | ❌ | — |
| Auto-dream consolidation | ✅ 4 files | ❌ | — |
| Context collapse | ✅ 3 files | ❌ | — |
| Team memory sync | ✅ 5 files | ❌ | — |
| CLAUDE.md 读取 | ✅ | ❌ | — |

---

## 15. Release / Platform

| 能力 | redcode | nocode | 差距 |
|------|---------|--------|------|
| CI pipeline | ✅ | ✅ GitHub Actions | — |
| 打包 / 安装 | ✅ install.sh | ❌ | — |
| Doctor 诊断 | ✅ | ❌ | — |
| Auto-updater | ✅ | ❌ | — |
| Native installer | ✅ 5 files | ❌ | — |
| Build variants | ✅ | ❌ | — |
| 灰度 / 回滚 | ✅ | ❌ | — |

---

## 16. nocode 独有能力（redcode 没有的）

| 能力 | 说明 |
|------|------|
| TUI 四窗格布局 | transcript / task list / task detail / events |
| TUI overlay 系统 | F1 help / F2 inspector / F3 permissions |
| TUI permission approve/deny | 实时 overlay 审批 |
| TUI adaptive poll | streaming 时 16ms，空闲 120ms |
| TUI async submission | engine move 进 thread，不阻塞渲染 |
| Process agent backoff profiles | IO/Decode/Exit 独立 backoff 策略 |
| response_result 统一命名 | 替代 structured_output 的新规范 |
| Capability matrix | 全 provider 能力对比矩阵 |

---

## 总结

### 量化对比

| 维度 | redcode | nocode | 覆盖率 |
|------|---------|--------|--------|
| 代码量 | 507,396 LOC | ~22,000 LOC | 4.3% |
| 文件数 | 1,910 | 43 | 2.3% |
| Tools | 42 | 6 | 14% |
| Commands | ~103 | ~30 | 29% |
| Providers | 6 (含 Bedrock/Vertex/Foundry) | 4 (含 Mock) | 67% |
| UI 组件 | 349 TSX | 4 .rs | — |
| Feature flags | 88 | 0 | 0% |
| Hooks | 76 | 0 | 0% |

### 按优先级排列的缺口

**P0 — 阻塞日常使用：**
1. Bash 权限沙箱（当前无任何安全限制）
2. Context compaction（长对话会爆 context）
3. CLAUDE.md 读取（无法加载项目指令）

**P1 — 阻塞替代 redcode：**
4. WebSearch / WebFetch tools
5. Agent tool（spawn sub-agent）
6. MCP 子系统（至少 client + tool execution）
7. /commit, /diff 等 git 命令
8. Permission 规则引擎
9. Task 持久化 + resume

**P2 — 产品完整性：**
10. Plugin / Skill 系统
11. IDE 集成 (VS Code)
12. Auth / login
13. Bedrock / Vertex provider
14. 打包 / 安装 / doctor

**P3 — 长尾：**
15. 88 个 feature flags
16. Telemetry
17. Voice mode
18. Onboarding
19. 349 个 Ink 组件的等价物
