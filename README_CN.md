# nocode

[English](README.md) | [Development Guide](docs/DEVELOPMENT.md)

终端原生 AI 编程助手。Rust 构建。38K LOC，51 模块，25 工具，555 测试。

## 安装

```bash
npm install -g @telagod/nocode
```

或从源码构建：

```bash
git clone https://github.com/telagod/nocode.git
cd nocode && cargo build --release
cp target/release/nocode ~/.local/bin/
```

## 配置

设置一个 API 密钥即可：

```bash
export ANTHROPIC_API_KEY="sk-ant-..."   # Claude（默认）
# 或
export OPENAI_API_KEY="sk-..."          # OpenAI
# 或
export GEMINI_API_KEY="..."             # Gemini
```

## 使用

```bash
nocode --repl                        # 交互式 REPL
nocode --tui                         # 四窗格终端 UI
nocode --status                      # 系统诊断
nocode --bridge-once "提示词"         # 单轮本地执行
nocode --bridge-remote-once "提示词"  # 单轮远程执行
```

## 提供商

根据环境变量自动检测。优先级：显式覆盖 > Google > OpenAI > Anthropic > Mock。

| 提供商 | API 密钥 | 默认模型 |
|--------|----------|----------|
| Anthropic (Claude) | `ANTHROPIC_API_KEY` | `claude-opus-4-6` |
| OpenAI (GPT) | `OPENAI_API_KEY` | `gpt-5.4` |
| Google (Gemini) | `GEMINI_API_KEY` | `gemini-3.1-pro` |
| Custom | `NOCODE_CUSTOM_BASE_URL` | 用户指定 |
| Mock | （无，兜底） | `sonnet` |

覆盖提供商或模型：

```bash
export NOCODE_MODEL_PROVIDER=anthropic
export NOCODE_MODEL=gpt-5.4
```

接入任何 OpenAI/Claude 兼容端点（Ollama、vLLM、LiteLLM 等）：

```bash
export NOCODE_MODEL_PROVIDER=custom
export NOCODE_CUSTOM_BASE_URL=http://localhost:11434/v1
export NOCODE_CUSTOM_API_FORMAT=openai   # anthropic (Messages API) | openai (Chat/Responses) | google (generateContent)
export NOCODE_MODEL=llama3
```

## REPL 命令

### 会话
| 命令 | 说明 |
|------|------|
| `/help` | 显示所有命令 |
| `/status` | 提供商和引擎状态 |
| `/runtime` | 运行时诊断 |
| `/history` | 对话历史 |
| `/inputs` | 原始输入历史 |
| `/quit` | 退出 |

### Git
| 命令 | 说明 |
|------|------|
| `/commit <消息>` | `git add -A && git commit -m "..."` |
| `/diff [参数]` | 执行 `git diff` |
| `/branch [分支名]` | 执行 `git branch` |

### 任务
| 命令 | 说明 |
|------|------|
| `/tasks [过滤]` | 列出任务。过滤：`all`、`completed`、`shell`、`agent`、`status:X type:Y` |
| `/task-shell <命令>` | 启动 shell 任务 |
| `/task-agent <agent-id> <提示词>` | 启动 agent 任务 |
| `/task-dream [会话数] [描述]` | 启动 dream 任务 |
| `/task-show <id\|first\|last\|latest\|prev\|next>` | 查看任务详情 |
| `/task-open` | 打开选中任务 |
| `/task-queue` | 查看任务队列 |
| `/task-run-next` | 运行下一个排队任务 |
| `/task-run-all` | 运行所有排队任务 |
| `/task-stop <task-id>` | 停止运行中的任务 |

### 团队
| 命令 | 说明 |
|------|------|
| `/team-create <子任务1; 子任务2; ...>` | 启动并行 agent 团队 |
| `/team-status` | 查看团队状态 |

### 编辑
| 命令 | 说明 |
|------|------|
| `/draft <文本>` | 开始草稿 |
| `/edit <文本>` | 替换草稿内容 |
| `/append <文本>` | 追加到草稿 |
| `/send` | 发送草稿 |
| `/queue <提示词>` | 排队一个提示词 |
| `/queue-slash </命令>` | 排队一个斜杠命令 |
| `/queue-show` | 查看排队项 |

### 导航
| 命令 | 说明 |
|------|------|
| `/focus <transcript\|tasks\|detail>` | 聚焦 TUI 窗格 |
| `/tasks-next` `/j` | 选择下一个任务 |
| `/tasks-prev` `/k` | 选择上一个任务 |
| `/enter` | 打开选中任务 |
| `/history-prev` `/history-next` | 浏览输入历史 |

### 账户
| 命令 | 说明 |
|------|------|
| `/login <api-key>` | 存储 API 密钥到 `~/.nocode/credentials` |
| `/logout` | 移除已存储的凭证 |
| `/doctor` | 系统诊断（提供商、工具、连通性） |
| `/plugin list` | 列出已发现的插件 |

## TUI

四窗格全屏界面，支持 Markdown 渲染（pulldown-cmark + syntect 语法高亮）、RGB 色彩、浮层系统。

| 按键 | 功能 |
|------|------|
| `Alt-1..4` | 聚焦窗格（对话 / 任务列表 / 任务详情 / 事件） |
| `Tab` / `Shift-Tab` | 循环切换窗格 |
| `Up` / `Down` | 滚动或导航 |
| `PgUp` / `PgDn` | 快速滚动 |
| `Ctrl-P` / `Ctrl-N` | 输入历史 |
| `Ctrl-U` | 清空输入 |
| `F1` / `?` | 帮助浮层 |
| `F2` | 检查器浮层 |
| `F3` | 权限浮层（`a` 批准 / `d` 拒绝） |
| `Esc` | 关闭浮层或退出 |

## 环境变量

| 变量 | 用途 |
|------|------|
| `NOCODE_MODEL_PROVIDER` | 强制提供商：`anthropic`、`openai`、`google`、`custom`、`mock` |
| `NOCODE_MODEL` | 覆盖模型名 |
| `NOCODE_CUSTOM_BASE_URL` | Custom 提供商端点 |
| `NOCODE_CUSTOM_API_FORMAT` | API 协议格式：`anthropic`（Messages API）、`openai`（Chat Completions / Responses）、`google`（generateContent） |
| `NOCODE_SYSTEM_PROMPT` | 覆盖系统提示词 |
| `NOCODE_MODEL_REASONING_EFFORT` | `low`、`medium`、`high` |
| `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `GEMINI_API_KEY` | 提供商 API 密钥 |
| `ANTHROPIC_MODEL` / `OPENAI_MODEL` / `GEMINI_MODEL` | 各提供商模型覆盖 |
| `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` | 各提供商端点覆盖 |
| `NOCODE_BRIDGE_BASE_URL` | 远程 bridge 端点 |
| `NOCODE_BRIDGE_AUTH_TOKEN` | 远程 bridge Bearer token |

## 支持平台

| 平台 | npm 包 |
|------|--------|
| Linux x64 | `@telagod/nocode-linux-x64` |
| Linux ARM64 | `@telagod/nocode-linux-arm64` |
| macOS x64 | `@telagod/nocode-darwin-x64` |
| macOS ARM64 | `@telagod/nocode-darwin-arm64` |
| Windows x64 | `@telagod/nocode-win32-x64` |
| Windows ARM64 | `@telagod/nocode-win32-arm64` |

## 许可证

MIT
