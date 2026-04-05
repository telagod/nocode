# nocode vs redcode 功能对齐清单

> 更新时间：2026-04-06（v0.2 完成后）
> 基准：redcode TS/Bun (507,396 LOC, 1,910 files) vs nocode Rust (~37,600 LOC, 43 files)

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
| Live SSE streaming | ✅ | 🔶 管道已通，TUI 异步渲染已搭 | 缺真实 API 端到端验证 |
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

## 2. Tool 层

| 工具 | redcode | nocode | 状态 |
|------|---------|--------|------|
| Read (FileRead) | ✅ | ✅ | 完成 |
| Edit (FileEdit) | ✅ | ✅ | 完成 |
| Write (FileWrite) | ✅ | ✅ | 完成 |
| Bash | ✅ 2,621 LOC | ✅ 含安全沙箱 | 完成 |
| Glob | ✅ | ✅ 基于 find | 完成 |
| Grep | ✅ | ✅ 基于 grep -rn | 完成 |
| WebSearch | ✅ | ✅ | 完成 |
| WebFetch | ✅ | ✅ | 完成 |
| Agent (spawn sub-agent) | ✅ | ✅ InProcessAgentHost | 完成 |
| MCP Tool | ✅ | ✅ mcp: 前缀动态分发 | 完成 |
| NotebookEdit | ✅ 490 LOC | ❌ | — |
| PowerShell | ✅ 2,049 LOC | ❌ | — |
| LSP Tool | ✅ 860 LOC | ❌ | — |
| ListMcpResources | ✅ | ❌ | — |
| ReadMcpResource | ✅ | ❌ | — |
| McpAuth | ✅ 215 LOC | ❌ | — |
| SkillTool | ✅ | ❌ | — |
| TaskCreate/Get/List/Update/Stop/Output | ✅ 6 tools | ❌ | — |
| TeamCreate/Delete | ✅ | ❌ (命令层有) | — |
| SendMessage | ✅ | ❌ | — |
| AskUserQuestion | ✅ | ❌ | — |
| TodoWrite | ✅ | ❌ | — |
| EnterPlanMode / ExitPlanMode | ✅ | ❌ | — |
| EnterWorktree / ExitWorktree | ✅ | ❌ | — |
| ConfigTool | ✅ | ❌ | — |
| ToolSearch | ✅ | ❌ | — |
| BriefTool | ✅ | ❌ | — |
| SleepTool | ✅ | ❌ | — |
| ScheduleCron / RemoteTrigger | ✅ | ❌ | — |
| SyntheticOutput | ✅ | ❌ | — |
| **合计** | **42 tools** | **10 tools** | **24%** |

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
| /model | ✅ | ❌ | — |
| /config | ✅ | ❌ | — |
| /permissions | ✅ | ❌ | — |
| /review, /ultrareview | ✅ | ❌ | — |
| /plan, /ultraplan | ✅ | ❌ | — |
| /compact | ✅ | ❌ | — |
| /memory | ✅ | ❌ | — |
| /agents | ✅ | ❌ | — |
| /skills | ✅ | ❌ | — |
| /mcp | ✅ | ❌ | — |
| /ide | ✅ | ❌ | — |
| /voice | ✅ | ❌ | — |
| /vim, /theme | ✅ | ❌ | — |
| /cost, /usage | ✅ | ❌ | — |
| /insights | ✅ | ❌ | — |
| /resume, /rewind | ✅ | ❌ | — |
| /export, /copy | ✅ | ❌ | — |
| /env, /keybindings | ✅ | ❌ | — |
| /bughunter, /security-review | ✅ | ❌ | — |
| /free | ✅ | 🚫 不迁 | 设计决策 |
| **合计** | **~103** | **~40** | **39%** |

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
| Permission 对话框 | ✅ 44 组件 | 🔶 F3 overlay | 缺完整 lifecycle |
| 虚拟滚动 | ✅ | ❌ | — |
| 代码高亮 | ✅ | ❌ | — |
| Diff 视图 | ✅ | ❌ | — |
| 搜索 (GlobalSearch, QuickOpen) | ✅ | ❌ | — |
| 主题系统 | ✅ | ❌ | — |
| Vim 模式 | ✅ | ❌ | — |
| 输入历史 | ✅ | ✅ | 完成 |
| 草稿/队列 | ✅ | ✅ | 完成 |
| Spinner / 进度条 | ✅ | ❌ | — |
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
| Task 持久化 (跨 session) | ✅ | 🔶 persist_task_record 存在 | 缺 resume |
| Task resume / 恢复 | ✅ | ❌ | — |
| Task 审计链 | ✅ | ❌ | — |
| Remote daemon | ✅ | ❌ | — |
| Agent swarm (21 files) | ✅ | ❌ | — |
| Inter-agent messaging | ✅ | ❌ | — |
| Agent creation wizard | ✅ | ❌ | — |
| Scheduled tasks / Cron | ✅ | ❌ | — |

---

## 6. Bridge / Session 层

| 能力 | redcode | nocode | 状态 |
|------|---------|--------|------|
| Local bridge | ✅ | ✅ | 完成 |
| Remote bridge (HTTP) | ✅ | 🔶 demo transport | 功能可用 |
| Session registry | ✅ | ❌ | — |
| Session resume | ✅ | 🔶 resume_with_reader | 缺产品化 |
| WebSocket 长连接 | ✅ | ❌ | — |
| Reconnect / heartbeat | ✅ | ❌ | — |
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
| Context compaction | ✅ 15 files | ✅ TruncatingCompactor | 完成（截断式） |
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

---

## 总结

### 量化对比

| 维度 | redcode | nocode | 覆盖率 |
|------|---------|--------|--------|
| 代码量 | 507,396 LOC | ~37,600 LOC | 7.4% |
| 文件数 | 1,910 | 43 | 2.3% |
| Tools | 42 | 10 | 24% |
| Commands | ~103 | ~40 | 39% |
| Providers | 6 | 5 (含 Custom + Gemini) | 83% |
| Feature flags | 88 | 0 | 0% |

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
10. Live streaming 端到端验证
11. IDE 集成 (VS Code server mode)
12. LLM-powered context compaction
13. Task resume from JSONL
14. Plugin execution runtime
15. Skill system

**P3 — 长尾：**
16. WebSocket bridge + reconnect
17. Session registry
18. Telemetry (opt-in)
19. Voice mode
20. Cross-platform packaging
