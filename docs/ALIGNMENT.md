# nocode vs Claude Code 功能对齐清单

> 更新时间：2026-04-08（v2 Phase 2 — TUI/Provider/Task/Bridge 深化后）
> 基准：Claude Code 2.1.90 (sdk-tools.d.ts 21 tools) vs nocode Rust (90 .rs files, 476 tests)

## 图例

- ✅ 已实现且可用
- 🔶 部分实现 / 骨架存在
- ❌ 未实现
- 🚫 明确不迁（设计决策）

---

## 1. Provider / Model 层

| 能力 | redcode | nocode | 状态 |
|------|---------|--------|------|
| Claude Messages API | ✅ @anthropic-ai/sdk | ✅ 原生 HTTP | 完成 |
| OpenAI Chat Completions | ✅ | ✅ 合并入 OpenAi | 完成 |
| OpenAI Responses | ✅ | ✅ 合并入 OpenAi（默认） | 完成 |
| Gemini API | ❌ | ✅ 原生 generateContent | nocode 独有 |
| Custom provider (任意端点) | ❌ | ✅ ApiFormat 路由 | nocode 独有 |
| AWS Bedrock | ✅ Bedrock SDK | 🔶 可通过 Custom + claude format | 需用户配置 SigV4 |
| Google Vertex AI | ✅ Vertex SDK | 🔶 可通过 Custom + claude format | 需用户配置 OAuth |
| Anthropic Foundry | ✅ Foundry SDK | ❌ | — |
| Mock provider | ✅ | ✅ | 完成 |
| Live SSE streaming | ✅ | ✅ SseReader + stall detection | 完成 |
| Tool call parsing (Claude) | ✅ | ✅ extract_claude_tool_calls | 完成 |
| Tool call parsing (OpenAI) | ✅ | ✅ extract_openai_tool_calls | 完成 |
| Tool call parsing (Gemini) | ❌ | ✅ extract_gemini_tool_calls | nocode 独有 |
| Tool execution loop | ✅ | ✅ runtime.rs 完整循环 | 完成 |
| Tools in API request body | ✅ | ✅ ToolSchema JSON | 完成 |
| Prompt caching | ✅ cache control | ❌ | — |
| Structured output / JSON schema | ✅ | 🔶 schema 字段存在 | 未产品化 |
| Cost tracking / token billing | ✅ | 🔶 UsageTracker 存在 | 缺 USD 计费 |
| Reasoning effort | ✅ | ✅ env var 支持 | 完成 |
| Thinking mode | ✅ extended thinking | 🔶 Adaptive/Disabled enum | 缺 thinkback |
| Model selection / fallback | ✅ | ✅ | 完成 |
| Retry / backoff | ✅ | ✅ 指数退避 | 完成 |
| `/free` embedded proxy | ✅ cursor2api | 🚫 明确不迁 | 设计决策 |

---

## 2. Tool 层 (21/21 — 100% Claude Code parity)

| 工具 | Claude Code | nocode | 状态 |
|------|------------|--------|------|
| Agent | ✅ | ✅ 后台线程 + model override | 完成 |
| AskUserQuestion | ✅ | ✅ 结构化问题 | 完成 |
| Bash | ✅ | ✅ JSON output + timeout + background + sandbox | 完成 |
| Config | ✅ | ✅ get/set/list | 完成 |
| EnterWorktree | ✅ | ✅ git worktree | 完成 |
| ExitPlanMode | ✅ | ✅ allowedPrompts | 完成 |
| ExitWorktree | ✅ | ✅ git worktree remove | 完成 |
| FileEdit | ✅ | ✅ replace + replace_all | 完成 |
| FileRead | ✅ | ✅ text + image base64 + notebook + PDF | 完成 |
| FileWrite | ✅ | ✅ create/overwrite | 完成 |
| Glob | ✅ | ✅ mtime sorted | 完成 |
| Grep | ✅ | ✅ rg flags (-B/-A/-C/-n/-i/type/offset/multiline) | 完成 |
| ListMcpResources | ✅ | ✅ server filter | 完成 |
| Mcp | ✅ | ✅ generic dispatch | 完成 |
| NotebookEdit | ✅ | ✅ replace/insert/delete cells | 完成 |
| ReadMcpResource | ✅ | ✅ resources/read via McpClient | 完成 |
| TaskOutput | ✅ | ✅ task_id/block/timeout | 完成 |
| TaskStop | ✅ | ✅ task_id/shell_id + PID kill | 完成 |
| TodoWrite | ✅ | ✅ batch write/replace | 完成 |
| WebFetch | ✅ | ✅ HTML strip + prompt context | 完成 |
| WebSearch | ✅ | ✅ DuckDuckGo + domain filter | 完成 |
| **合计** | **21 tools** | **21 tools** | **100%** |

---

## 3. Command 层 (Slash Commands)

| 命令类别 | redcode | nocode | 状态 |
|----------|---------|--------|------|
| 基础 (/help, /quit, /status) | ✅ | ✅ | 完成 |
| 历史 (/history, /inputs) | ✅ | ✅ | 完成 |
| 任务管理 (/tasks, /task-*) | ✅ | ✅ | 完成 |
| 队列 (/queue, /queue-slash) | ✅ | ✅ | 完成 |
| 草稿 (/draft, /edit, /append, /send) | ✅ | ✅ | 完成 |
| 焦点 (/focus) | ✅ | ✅ | 完成 |
| Git (/commit, /diff, /branch) | ✅ | ✅ | 完成 |
| 团队 (/team-create, /team-status) | ✅ | ✅ | 完成 |
| 账户 (/login, /logout) | ✅ | ✅ | 完成 |
| 诊断 (/doctor) | ✅ | ✅ | 完成 |
| 插件 (/plugin list) | ✅ | ✅ 骨架 | 完成 |
| /model | ✅ | ✅ | 完成 |
| /config | ✅ | ✅ overlay | 完成 |
| /permissions | ✅ | ✅ | 完成 |
| /review, /ultrareview | ✅ | ❌ | — |
| /plan, /ultraplan | ✅ | ❌ | — |
| /compact | ✅ | ✅ TailCompactor + RichCompactor | 完成 |
| /memory | ✅ | ✅ overlay | 完成 |
| /agents | ✅ | ✅ overlay | 完成 |
| /skills | ✅ | ❌ | — |
| /mcp | ✅ | ✅ overlay | 完成 |
| /ide | ✅ | ❌ | — |
| /voice | ✅ | ❌ | — |
| /vim, /theme | ✅ | ✅ | 完成 |
| /cost, /usage | ✅ | ✅ overlay | 完成 |
| /insights | ✅ | ❌ | — |
| /resume, /rewind | ✅ | ✅ /resume | 完成 |
| /export, /copy | ✅ | ✅ /export | 完成 |
| /env, /keybindings | ✅ | ❌ | — |
| /bughunter, /security-review | ✅ | ❌ | — |
| /free | ✅ | 🚫 不迁 | 设计决策 |
| **合计** | **~103** | **~52** | **50%** |

---

## 4. UI / 交互层

| 能力 | redcode | nocode | 状态 |
|------|---------|--------|------|
| Ink/React 富文本 REPL | ✅ 349 TSX, 77K LOC | ❌ 纯文本 REPL | 架构不同 |
| TUI 多窗格 | ❌ | ✅ 四窗格 crossterm | nocode 独有 |
| TUI overlay 系统 | ❌ | ✅ F1/F2/F3 | nocode 独有 |
| TUI async submission | ❌ | ✅ engine move 进 thread | nocode 独有 |
| TUI adaptive poll | ❌ | ✅ 16ms streaming / 120ms idle | nocode 独有 |
| TUI permission approve/deny | ❌ | ✅ F3 overlay | nocode 独有 |
| 消息类型渲染 (38 种) | ✅ | 🔶 role badge + 行级着色 | 缺富文本 |
| Permission 对话框 | ✅ 44 组件 | ✅ y/n/a overlay + TuiPermissionBridge | 完成 |
| 虚拟滚动 | ✅ | ✅ height cache + sticky scroll | 完成 |
| 代码高亮 | ✅ | ✅ syntect + pulldown-cmark | 完成 |
| Diff 视图 | ✅ | ❌ | — |
| 搜索 (GlobalSearch, QuickOpen) | ✅ | ❌ | — |
| 主题系统 | ✅ | ✅ dark/light + Ctrl-T toggle | 完成 |
| Vim 模式 | ✅ | ✅ h/j/k/l/w/b/x/0/$/I/A | 完成 |
| 输入历史 | ✅ | ✅ | 完成 |
| 草稿/队列 | ✅ | ✅ | 完成 |
| Spinner / 进度条 | ✅ | ✅ Spinner + stall detection | 完成 |
| 剪贴板集成 | ✅ | ❌ | — |
| Onboarding 引导 | ✅ | ❌ | — |
| 自动更新 | ✅ | ❌ | — |

---

## 5. Task / Agent 层

| 能力 | redcode | nocode | 状态 |
|------|---------|--------|------|
| Shell task | ✅ | ✅ | 完成 |
| Agent task (in-process) | ✅ | ✅ InProcessAgentHost | 完成 |
| Agent task (process) | ✅ | ✅ ProcessTaskAgentHost | 完成 |
| Dream task | ✅ | ✅ | 完成 |
| Daemon supervisor | ✅ | ✅ restart/backoff/failure | 完成 |
| Team create (并行多 agent) | ✅ | ✅ /team-create | 完成 |
| Task 持久化 (跨 session) | ✅ | ✅ JSONL persist/load | 完成 |
| Task resume / 恢复 | ✅ | ✅ load_from_file | 完成 |
| Task 审计链 | ✅ | ✅ TaskEvent lifecycle recording | 完成 |
| Remote daemon | ✅ | ❌ | — |
| Agent swarm (21 files) | ✅ | ❌ | — |
| Inter-agent messaging | ✅ | ✅ WorkerRegistry inbox | 完成 |
| Agent creation wizard | ✅ | ❌ | — |
| Scheduled tasks / Cron | ✅ | ❌ | — |

---

## 6. Bridge / Session 层

| 能力 | redcode | nocode | 状态 |
|------|---------|--------|------|
| Local bridge | ✅ | ✅ | 完成 |
| Remote bridge (HTTP) | ✅ | ✅ /v1/query + agentic loop | 完成 |
| Session registry | ✅ | ✅ SessionRegistry + meta.json | 完成 |
| Session resume | ✅ | ✅ JSONL + meta.json | 完成 |
| WebSocket 长连接 | ✅ | ❌ | — |
| Reconnect / heartbeat | ✅ | ✅ ConnectionRegistry + timeout sweep | 完成 |
| Permission callback | ✅ | ✅ wire 已通 | 完成 |
| Bridge event streaming | ✅ | ✅ BridgeEventWire | 完成 |

---

## 7. Persistence / 存储层

| 能力 | redcode | nocode | 状态 |
|------|---------|--------|------|
| Session persistence | ✅ | ✅ JSONL | 完成 |
| Transcript persistence | ✅ | ✅ | 完成 |
| History store | ✅ | ✅ | 完成 |
| File history | ✅ | ✅ | 完成 |
| Resume snapshot | ✅ | ✅ | 完成 |
| Task record persistence | ✅ | ✅ persist_task_record | 完成 |
| Secure storage | ✅ | ❌ | — |
| Settings sync | ✅ | ❌ | — |

---

## 8. Permission / 安全层

| 能力 | redcode | nocode | 状态 |
|------|---------|--------|------|
| Tool permission gating | ✅ 24 files | ✅ ToolPermissionContext + PermissionRule | 完成 |
| Bash 权限沙箱 | ✅ 2,621 LOC | ✅ check_bash_safety() | 完成 |
| Permission rules engine | ✅ | ✅ CommandContains + ArgumentContains | 完成 |
| 9 条预设安全规则 | — | ✅ rm -rf, mkfs, dd, shutdown 等 | nocode 独有 |
| Bash 只读验证 | ✅ 1,990 LOC | ❌ | — |
| PowerShell 路径验证 | ✅ 2,049 LOC | ❌ | — |
| Permission rules UI | ✅ 1,178 LOC | ❌ | — |
| Auto-approve / deny | ✅ | ✅ bridge 层 | 完成 |
| Classifier approvals | ✅ | ❌ | — |
| TUI permission overlay | ❌ | ✅ F3 approve/deny | nocode 独有 |

---

## 9. MCP (Model Context Protocol)

| 能力 | redcode | nocode | 状态 |
|------|---------|--------|------|
| MCP client | ✅ | ✅ McpClient JSON-RPC | 完成 |
| MCP tool discovery | ✅ | ✅ list_tools() | 完成 |
| MCP tool execution | ✅ | ✅ call_tool() + mcp: 分发 | 完成 |
| MCP auth (OAuth) | ✅ 2,465 LOC | ❌ | — |
| MCP resource listing/reading | ✅ | ❌ | — |
| MCP server management | ✅ 12 components | ❌ | — |
| MCP elicitation | ✅ 1,168 LOC | ❌ | — |
| In-process transport | ✅ | ❌ | — |
| VS Code MCP | ✅ | ❌ | — |

---

## 10. Plugin / Skill 系统

| 能力 | redcode | nocode | 状态 |
|------|---------|--------|------|
| Plugin manifest discovery | ✅ | ✅ manifest.json 骨架 | 完成 |
| Plugin 安装/管理 | ✅ 44 files | ❌ | — |
| Plugin execution runtime | ✅ | ❌ | — |
| Skill 系统 | ✅ | ❌ | — |
| Plugin CLI commands | ✅ | 🔶 /plugin list | 骨架 |

---

## 11. IDE / 外部集成

| 能力 | redcode | nocode | 状态 |
|------|---------|--------|------|
| VS Code 集成 | ✅ | ❌ | — |
| JetBrains 集成 | ✅ | ❌ | — |
| IDE server mode | ✅ | ❌ | v0.3 计划 |
| Desktop app | ✅ | ❌ | — |
| Mobile | ✅ | ❌ | — |
| Deep linking | ✅ | ❌ | — |

---

## 12. Auth / 账户

| 能力 | redcode | nocode | 状态 |
|------|---------|--------|------|
| /login /logout | ✅ | ✅ 凭证存储 | 完成 |
| OAuth client | ✅ | ❌ | — |
| API key verification | ✅ | ❌ (直接用 env var) | — |
| Session ingress auth | ✅ | ❌ | — |
| Referral / passes | ✅ | ❌ | — |

---

## 13. Analytics / Telemetry

| 能力 | redcode | nocode | 状态 |
|------|---------|--------|------|
| Feature flags | ✅ 88 flags | ❌ | — |
| Datadog integration | ✅ | ❌ | — |
| Event logging | ✅ | ❌ | — |
| Metrics opt-out | ✅ | ❌ | — |

---

## 14. Context / Memory

| 能力 | redcode | nocode | 状态 |
|------|---------|--------|------|
| Context compaction | ✅ 15 files | ✅ TailCompactor + RichCompactor (LLM-powered) | 完成 |
| CLAUDE.md 读取 | ✅ | ✅ discover + format | 完成 |
| Session memory | ✅ | ❌ | — |
| Memory extraction | ✅ | ❌ | — |
| Auto-dream consolidation | ✅ | ❌ | — |
| Context collapse | ✅ | ❌ | — |
| Team memory sync | ✅ | ❌ | — |

---

## 15. Release / Platform

| 能力 | redcode | nocode | 状态 |
|------|---------|--------|------|
| CI pipeline | ✅ | ✅ GitHub Actions | 完成 |
| install.sh | ✅ | ✅ | 完成 |
| /doctor 诊断 | ✅ | ✅ | 完成 |
| Auto-updater | ✅ | ❌ | — |
| Native installer | ✅ | ❌ | — |
| Build variants | ✅ | ❌ | — |
| 灰度 / 回滚 | ✅ | ❌ | — |

---

## 16. nocode 独有能力

| 能力 | 说明 |
|------|------|
| Gemini 原生支持 | 第一方 generateContent API |
| Custom provider + ApiFormat | 任意端点，claude/openai/gemini 协议路由 |
| TUI 四窗格布局 | transcript / task list / task detail / events |
| TUI overlay 系统 | F1 help / F2 inspector / F3 permissions |
| TUI permission approve/deny | 实时 overlay 审批 |
| TUI adaptive poll | streaming 时 16ms，空闲 120ms |
| TUI async submission | engine move 进 thread，不阻塞渲染 |
| 三格式工具调用解析 | Claude tool_use + OpenAI function_call + Gemini functionCall |
| MCP JSON-RPC client | stdio 传输，工具发现 + 执行 |
| Process agent backoff profiles | IO/Decode/Exit 独立 backoff 策略 |
| Bash 安全沙箱 | 9 条预设规则，check_bash_safety() |
| response_result 统一命名 | 替代 structured_output 的新规范 |
| Capability matrix | 全 provider 能力对比矩阵 |
| ErrorKind 细粒度分类 | 10 种错误类型 (Auth/RateLimit/Quota/Timeout/ServerError 等) |
| SSE stream state machine | SseReader 通用解析器 + stall detection |
| Task cancel token | AtomicBool 协作式取消 + agentic loop 检查 |
| Task audit trail | TaskEvent 生命周期事件记录 |
| Bridge HTTP service | /v1/query + /v1/sessions + /v1/connect + heartbeat |
| Session meta persistence | meta.json + SessionRegistry 全局索引 |
| Message backgrounds | 按 role 着色消息背景 (user/assistant/tool/error) |
| TUI modular split | overlays/commands/events 子模块拆分 |

---

## 总结

### 量化对比

| 维度 | Claude Code | nocode | 覆盖率 |
|------|------------|--------|--------|
| Tools | 21 | 21 | 100% |
| Tool schemas | sdk-tools.d.ts | 严格对齐 | 100% |
| Providers | 1 (Claude) | 5 (Claude/OpenAI/Gemini/Custom/Mock) | 超集 |
| Run modes | ~5 | 9 | 超集 |
| Slash commands | ~20 | 23 | 超集 |
| Tests | — | 476 | — |
| Modules | — | 90 .rs | — |

### 与 v0.1 对比的进展

| 维度 | v0.1 | v0.2 | 变化 |
|------|------|------|------|
| Tools | 6 | 10 | +4 (WebFetch, WebSearch, Agent, MCP) |
| Commands | ~30 | ~40 | +10 (git, team, account, doctor, plugin) |
| Providers | 4 (含 Bedrock/Vertex) | 5 (含 Gemini/Custom) | 重构简化 |
| Tool call parsing | ❌ | ✅ 三格式 | 新增 |
| MCP client | ❌ | ✅ | 新增 |
| Bash sandbox | ❌ | ✅ | 新增 |
| CLAUDE.md | ❌ | ✅ | 新增 |
| Permission rules | ❌ | ✅ | 新增 |

### 按优先级排列的剩余缺口

**P0 — 已全部完成：**
1. ~~Bash 权限沙箱~~ ✅
2. ~~Context compaction~~ ✅
3. ~~CLAUDE.md 读取~~ ✅

**P1 — 已全部完成：**
4. ~~WebSearch / WebFetch tools~~ ✅
5. ~~Agent tool~~ ✅
6. ~~MCP 客户端~~ ✅
7. ~~Git 命令~~ ✅
8. ~~Permission 规则引擎~~ ✅
9. ~~Task 持久化~~ ✅ (记录层)

**P2 — 产品完整性（下一阶段）：**
10. ~~Live streaming 端到端验证~~ ✅ SseReader + stall detection
11. IDE 集成 (VS Code server mode)
12. ~~LLM-powered context compaction~~ ✅ RichCompactor
13. ~~Task resume from JSONL~~ ✅ load_from_file
14. Plugin execution runtime
15. Skill system

**P3 — 长尾：**
16. WebSocket bridge + reconnect
17. ~~Session registry~~ ✅ SessionRegistry + meta.json
18. Telemetry (opt-in)
19. Voice mode
20. Cross-platform packaging
