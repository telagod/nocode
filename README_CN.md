# nocode

快速、原生的终端 AI 编程助手。Rust 构建。

## 什么是 nocode？

nocode 是一个终端原生的 AI 助手，能与你并肩读写和运行代码。直连 Claude、OpenAI 或 Gemini — 无代理、无封装、无 Electron。任何 OpenAI 兼容或 Claude 兼容的端点都可以通过 Custom provider 接入。

```bash
# 安装
./install.sh

# 开始编程
export ANTHROPIC_API_KEY="sk-ant-..."
nocode --repl
```

## 核心特性

**10 个内置工具** — Read、Edit、Write、Bash、Glob、Grep、WebFetch、WebSearch、Agent、MCP

**5 个模型提供商** — Claude、OpenAI（Chat + Responses）、Gemini、Custom、Mock

**双界面** — 行模式 REPL 或四窗格全功能 TUI，支持彩色渲染和键盘导航

**多智能体** — 用 `/team-create` 并行启动多个 agent，用 `/team-status` 监控进度

**默认安全** — Bash 沙箱拦截危险命令，权限规则引擎管控工具访问

**CLAUDE.md 支持** — 自动发现项目指令：`CLAUDE.md`、`.claude/CLAUDE.md`、`.claude/rules/*.md`、`CLAUDE.local.md`

**MCP 支持** — JSON-RPC over stdio 客户端，通过 `mcp:` 前缀发现和执行工具

**工具调用循环** — 解析 Claude `tool_use`、OpenAI `function_call`、Gemini `functionCall` 响应中的工具调用，执行后将结果回传模型

## 快速开始

```bash
# 从源码构建
git clone https://github.com/telagod/nocode.git
cd nocode/rust
cargo build --release
./target/release/nocode --repl

# 或使用安装脚本
./install.sh
```

设置 API 密钥并启动：

```bash
export ANTHROPIC_API_KEY="sk-ant-..."   # 或 OPENAI_API_KEY 或 GEMINI_API_KEY
nocode --repl          # 行模式 REPL
nocode --tui           # 全功能终端 UI
nocode --status        # 系统诊断
```

## 运行模式

| 参数 | 说明 |
|------|------|
| `--repl` | 交互式 REPL，支持命令历史和流式输出 |
| `--tui` | 四窗格 TUI：对话记录、任务列表、任务详情、事件日志 |
| `--status` | 输出系统状态和提供商能力矩阵 |
| `--bridge-once "提示词"` | 单轮本地 bridge 执行 |
| `--bridge-remote-once "提示词"` | 单轮远程 HTTP bridge 执行 |
| `--process-agent-daemon` | 进程 agent 守护模式（内部） |
| `--process-agent-host` | 进程 agent 宿主模式（内部） |

## 命令一览

**会话**：`/help` `/status` `/runtime` `/history` `/quit`

**Git**：`/commit <消息>` `/diff [参数]` `/branch [分支名]`

**任务**：`/task-shell <命令>` `/task-agent <id> <提示词>` `/task-dream` `/tasks` `/task-show`

**团队**：`/team-create <子任务1; 子任务2; ...>` `/team-status`

**账户**：`/login <密钥>` `/logout` `/doctor`

**编辑**：`/draft` `/edit` `/append` `/send` `/queue`

**导航**：`/focus <窗格>` `/tasks-next` `/tasks-prev`

**插件**：`/plugin list`

## TUI 快捷键

| 按键 | 功能 |
|------|------|
| `Alt-1..4` | 聚焦 对话记录 / 任务列表 / 任务详情 / 事件日志 |
| `Tab` / `Shift-Tab` | 循环切换窗格 |
| `Up/Down` | 滚动或导航 |
| `PgUp/PgDn` | 快速滚动 |
| `Ctrl-P/N` | 输入历史 |
| `Ctrl-U` | 清空输入 |
| `F1` / `?` | 帮助浮层 |
| `F2` | 检查器浮层 |
| `F3` | 权限浮层（`a` 批准 / `d` 拒绝） |
| `Esc` | 关闭浮层或退出 |

## 提供商配置

nocode 根据环境变量自动检测提供商：

| 提供商 | 必需变量 | 默认模型 |
|--------|----------|----------|
| Claude | `ANTHROPIC_API_KEY` | `claude-sonnet-4-20250514` |
| OpenAI | `OPENAI_API_KEY` | `gpt-4.1` |
| Gemini | `GEMINI_API_KEY` | `gemini-2.5-flash` |
| Custom | `NOCODE_MODEL_PROVIDER=custom` | （用户指定） |

可通过 `NOCODE_MODEL_PROVIDER` 和 `NOCODE_MODEL` 覆盖。

其他环境变量：

| 变量 | 用途 |
|------|------|
| `NOCODE_MODEL` | 覆盖任意提供商的模型名 |
| `NOCODE_MODEL_PROVIDER` | 强制指定提供商：`claude`、`openai`、`gemini`、`custom`、`mock` |
| `NOCODE_CUSTOM_BASE_URL` | Custom 提供商的端点 URL |
| `NOCODE_CUSTOM_API_FORMAT` | Custom 的协议格式：`claude`、`openai` 或 `gemini` |
| `NOCODE_SYSTEM_PROMPT` | 覆盖默认系统提示词 |
| `NOCODE_MODEL_REASONING_EFFORT` | 推理力度：`low`、`medium`、`high` |
| `ANTHROPIC_MODEL` / `OPENAI_MODEL` / `GEMINI_MODEL` | 各提供商模型覆盖 |
| `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` | 各提供商端点覆盖 |

自动检测优先级：`NOCODE_MODEL_PROVIDER` 覆盖 → `GEMINI_API_KEY` → `OPENAI_API_KEY` → `ANTHROPIC_API_KEY` → Mock。

## 架构

```
nocode（CLI 二进制 — crates/nocode）
  src/main.rs        — 入口、提供商检测、引导配置
  src/repl.rs        — REPL 会话、约 40 个斜杠命令、任务管理
  src/tui.rs         — 基于 crossterm 的四窗格 TUI、异步流式、浮层系统
  src/claudemd.rs    — CLAUDE.md 发现与加载
  src/task_panel.rs  — 任务过滤与渲染

nocode-core（核心库 — crates/nocode-core）
  provider.rs          — ModelProvider（Claude/OpenAI/Gemini/Custom/Mock）、请求/响应/流式
  provider_transport.rs — HTTP 客户端、SSE 解析、重试/退避、各提供商鉴权
  query_engine.rs      — 对话生命周期、工具 schema 生成
  query_loop.rs        — 轮次执行、预算、停止钩子
  tool_execution/      — 10 个工具：Read/Edit/Write/Bash/Glob/Grep/WebFetch/WebSearch/Agent/MCP
  tool_registry.rs     — 工具注册、权限规则引擎
  task_runtime.rs      — shell/agent/dream 任务、进程守护 supervisor
  bridge_runtime.rs    — 本地/远程 bridge、权限回调
  session_persistence.rs — JSONL 会话/对话记录/历史/任务持久化
  mcp_client.rs        — MCP JSON-RPC 客户端（stdio）
  budget.rs            — token 预算跟踪与决策
  query_config.rs      — 查询配置与工具定义
  query_deps.rs        — 依赖注入、上下文压缩
```

## 路线图

### v0.1 — 已完成

- [x] 对话引擎：完整生命周期与工具循环
- [x] 5 个提供商适配器（Claude、OpenAI、Gemini、Custom、Mock）
- [x] API 请求中的工具调用（请求体包含 tools JSON schema）
- [x] 10 个工具（Read、Edit、Write、Bash、Glob、Grep、WebFetch、WebSearch、Agent、MCP）
- [x] REPL 约 40 个斜杠命令
- [x] TUI 四窗格、彩色渲染、浮层系统
- [x] CLAUDE.md 自动发现（用户/项目/规则/本地）
- [x] Bash 安全沙箱 + 权限规则引擎（9 条预设规则）
- [x] 上下文压缩（截断式 compactor）
- [x] 任务运行时：shell、agent、dream、进程守护 supervisor
- [x] Bridge：本地 + 远程 HTTP 传输
- [x] 会话/对话记录/历史持久化（JSONL）
- [x] 团队 agent：`/team-create` 并行多智能体
- [x] Git 命令：`/commit` `/diff` `/branch`
- [x] 账户：`/login` `/logout` 凭证存储
- [x] `/doctor` 系统诊断
- [x] 插件骨架：manifest.json 发现
- [x] CI 流水线（GitHub Actions：fmt、clippy、test、release build）
- [x] install.sh 打包

### v0.2 — 已完成

- [x] 模型响应中的工具调用解析（Claude `tool_use`、OpenAI `function_call`、Gemini `functionCall`）
- [x] 工具执行循环 — 模型请求工具，引擎执行并回传结果
- [x] MCP 客户端实现（JSON-RPC over stdio，工具发现与执行）
- [x] Provider 简化：6 → 5 个提供商，移除 Bedrock/Vertex 一等公民地位，新增 Gemini + Custom
- [x] ApiFormat 枚举：Custom 提供商的协议格式路由（Claude/OpenAI/Gemini）

### v0.3 — 下一步

- [ ] TUI 实时流式输出（真实 API 端到端验证）
- [ ] IDE 服务器模式（`--ide-server` JSON-RPC，支持 VS Code/JetBrains）
- [ ] 基于 LLM 的上下文压缩（摘要式）
- [ ] 从持久化 JSONL 恢复任务状态
- [ ] WebSocket bridge 传输，支持重连/心跳
- [ ] 会话注册表与远程会话恢复

### 远期

- [ ] 插件执行运行时（不仅是发现）
- [ ] Skill 系统
- [ ] 语音输入模式
- [ ] 引导流程
- [ ] 遥测（可选）
- [ ] 跨平台打包（macOS、Windows）

## 测试

```bash
cargo test                                      # 30 个测试
cargo clippy --all-targets -- -D warnings       # lint
cargo fmt --check                               # 格式检查
```

## 许可证

MIT
