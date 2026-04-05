# nocode

## 模块定位

`rust/` 是 `redcode` 的 Rust 重写主线，产物名固定为 `nocode`。

目标不是继续给 TS/Bun 版打补丁，而是把最核心的五层独立出来：

- query / provider kernel
- task runtime
- bridge / session
- standalone TUI
- packaging / release

`nocode` 明确不继承 `redcode` 的 `/free`、embedded proxy、feature-flag 大杂烩路线。

## 与 redcode 的关系

根项目 [README.md](/home/telagod/project/redcode/README.md) 描述的是当前 TS/Bun 版 `redcode`：

- Bun + TypeScript + React/Ink CLI
- 带 `/free` 与 embedded proxy
- 大量 feature flags、plugins、voice、GrowthBook 相关遗留面
- 以现有 CLI 行为和实验功能为主

`nocode` 的方向不同：

- Rust workspace，走 `cargo`
- provider 直连 adapter，不保留 `/free`
- 先做稳定内核与独立运行，再追 UI 对齐
- 先收口 query/task/bridge/TUI 主链，再考虑 feature parity

## 结果术语约定

`nocode` 对外只使用一套结果命名：

- CLI status / REPL diagnostics / bridge summary 使用 `response-result`
- Rust struct field / wire JSON field 使用 `response_result`
- task panel / session transcript / inspect detail 统一展示为 `result` 或 `response result`

`structured_output` 现在只保留在两类兼容层：

- provider 的 JSON schema request 名与 `structured_output_failure` 错误分类
- bridge / process-agent wire 的 backward-compatible decode alias

若看到 `structured_output`，默认应理解为 legacy/provider-internal 术语，而不是新的公共结果面。

## 当前状态

`nocode` 已经不是骨架，当前已具备：

- `nocode-core`：typed message model、assistant/model response、query loop、tool execution、session persistence、resume、bridge wire、task runtime。
- provider adapter：兼容 `Claude Messages`、`OpenAI Chat Completions`、`OpenAI Responses`，已有真实 HTTP transport、retry/backoff、SSE body parser。
- task runtime：已能驱动 shell task、in-process agent、process agent、dream task。
- bridge：已有 local/remote `SessionRunner`、permission callback 回传、HTTP remote transport。
- CLI/TUI：已有 `--status`、`--repl`、`--tui`、`--bridge-once`、`--bridge-remote-once`，以及最小可操作 TUI。

## 对照 redcode

| 面向 | `redcode` 现状 | `nocode` 现状 | 主要差距 |
| --- | --- | --- | --- |
| Runtime | Bun/TS 单体 CLI | Rust workspace + `nocode`/`nocode-core` | 发布与兼容链未收口 |
| Provider | Anthropic 主线 + `/free` 代理变体 | Claude/OpenAI typed adapter | 还缺 live chunk streaming 与更完整 capability/error matrix |
| Task | TS 任务/UI/实验功能深度耦合 | shell/agent/dream/process host 已独立 | 还缺持久任务表、remote daemon、取消/审计闭环 |
| Bridge | 远控桥、会话与前端耦合更深 | local/remote runner + HTTP transport demo | 还缺真正的 bridge service、resume、reconnect |
| UI | Ink REPL、对话框、权限浮层、较完整操作感 | standalone TUI 四窗格 | 还缺 permission overlay、live refresh、rich renderer |
| Platform | install/build/flags/plugins/voice/onboarding 完整遗留面 | Cargo workspace | 还缺 doctor、打包、CI、灰度/回滚标准 |

## 判定

当前版本适合：

- 内部预览
- 独立验证 provider / task / bridge / TUI 主链
- 继续替代 TS runtime 的迁移开发

当前版本还不适合：

- 直接上线替代 `redcode`
- 宣称与 TS Ink runtime 功能对齐
- 跨机稳定 bridge/daemon 部署

根因不是“没有界面”，而是以下硬差距仍未收口：

1. provider 还不是 live chunk streaming transport。
2. task system 还不是可恢复、可审计、可远端运行的 daemon/service。
3. bridge 还不是完整远端服务。
4. TUI 还没覆盖 permission/runtime/session 深水交互。
5. release / CI / compat 还没形成产品化闭环。

## 快速开始

```bash
cd rust
cargo run -p nocode -- --status
cargo run -p nocode -- --repl
cargo run -p nocode -- --tui
cargo run -p nocode -- --bridge-once "bridge rewrite"
cargo run -p nocode -- --bridge-remote-once "remote bridge rewrite"
cargo test -q
```

## 文档入口

- [DESIGN.md](/home/telagod/project/redcode/rust/DESIGN.md)：完整迁移设计、redcode 对照与长 TODO。
- [README.md](/home/telagod/project/redcode/README.md)：当前 TS/Bun 版 `redcode` 的产品说明。
- [roadmap.rs](/home/telagod/project/redcode/rust/crates/nocode-core/src/roadmap.rs)：CLI `--status` 使用的内建迁移路线图。

## TUI 已有能力

`cargo run -p nocode -- --tui` 当前提供：

- 顶部 `status` / `diagnostics` 双状态条
- transcript / task list / task detail / events 四面板
- ASCII box pane chrome、active pane 高亮、viewport 计数
- pane focus / pane scroll / Alt-1..4 快速切 pane
- task inspect / filter / open / auto-drive 刷新
- slash routing
- 输入历史、draft、queued command、origin 区分
- footer help / queue / editor strip
- overlay：`?`/`F1` help、`F2` inspector、`F3` permission stub

当前 TUI / REPL 已能直接按结果面操作：

- `/tasks result:agent`
- `/tasks result:shell`
- `/tasks result-structured:yes`
- `/tasks result-structured:no`
- `/task-show first`
- `/task-show last`

常用过滤语法：

| 命令 | 含义 |
| --- | --- |
| `/tasks result:agent` | 只看 agent 结果任务 |
| `/tasks result:shell` | 只看 shell 结果任务 |
| `/tasks result:dream` | 只看 dream 结果任务 |
| `/tasks result-structured:yes` | 只看带 `response_result` payload 的任务 |
| `/tasks result-structured:no` | 只看没有 `response_result` payload 的任务 |
| `/task-show first` | 打开当前过滤集里的首个任务 |
| `/task-show last` | 打开当前过滤集里的最后任务 |

常用按键：

| 按键 | 含义 |
| --- | --- |
| `Tab` / `Shift-Tab` | 切 pane |
| `Alt-1..4` | 直达 transcript / task list / task detail / events |
| `Up` / `Down` | transcript / events 滚动，task pane 选择移动 |
| `PgUp` / `PgDn` / `Home` / `End` | 当前 pane 快速滚动 |
| `Ctrl-P` / `Ctrl-N` | prompt history 上下翻 |
| `Ctrl-A` / `Ctrl-E` | input 光标头尾 |
| `Ctrl-U` | 清空当前 input |
| `Ctrl-L` | 清空 events pane |
| `?` / `F1` | 打开 help overlay |
| `F2` | 打开 inspector overlay |
| `F3` | 打开 permission overlay stub |

它现在已不是“只有四块文本”的最小壳层，但仍未达到 TS/Ink 版完整等价。当前明确缺口：

- 真正的 permission request lifecycle 尚未抬到 TUI，只留了 overlay 骨架
- transcript / tool / progress renderer 仍是文本视图，不是 TS 版 message component
- keybinding / footer pills / plugins / onboarding 还没迁完

## 状态与结果样例

### `--status`

`cargo run -p nocode -- --status` 的摘要行会把 provider、transport、tool turn 与结果面压成一行：

```text
status summary: provider=mock caps=stream(request=yes,live=no,sse=no) tool-use=no json-schema=no matrix=mock[stream(request=yes,live=no,sse=no) tool-use=no json-schema=no] | claude-messages[stream(request=yes,live=yes,sse=yes) tool-use=no json-schema=no] | openai-chat-completions[stream(request=yes,live=yes,sse=yes) tool-use=no json-schema=yes] | openai-responses[stream(request=yes,live=yes,sse=yes) tool-use=no json-schema=yes] model=mock-model transport=mock://nocode/mock(Post) headers=1 body={"provider":"mock","model":"mock-model","reply_target":"status demo","stream":false} stream-events=0 tools=0 turn-count=0 response-result=none error=none
```

`response-result=` 的解释：

- `none`：本次会话没有结构化结果
- JSON preview：本次会话拿到了 `response_result`
- 不再使用 `structured-output=...`

`caps=` 与 `matrix=` 的解释：

| 字段 | 含义 |
| --- | --- |
| `stream(request=yes|no,live=yes|no,sse=yes|no)` | 是否支持请求级 streaming、当前 adapter 是否有 live stream 路径、底层是否使用 SSE transport |
| `tool-use=yes|no` | 当前 provider adapter 是否显式支持模型级 tool use |
| `json-schema=yes|no` | 是否支持 request 侧 JSON schema / structured result 输出 |
| `matrix=` | 全 provider capability matrix；用于对比当前 provider 与其他 provider 的差异 |

`error=` 的解释：

| 值 | 含义 |
| --- | --- |
| `none` | 本次请求没有 provider/model error |
| `kind:class:retryable` | `kind` 为错误分类，`class` 为 HTTP status class，最后一位为是否可重试 |

示例：

```text
error=http_status:rate_limited:true
error=invalid_provider_response:none:false
error=structured_output_failure:none:false
```

### REPL diagnostics

REPL / TUI transcript 顶部 diagnostics 区会单独给出结果摘要与 pretty block：

```text
provider diagnostics:
provider=openai-responses model=gpt-4.1 caps=stream(request=yes,live=yes,sse=yes) tool-use=no json-schema=yes
capability-matrix=mock[stream(request=yes,live=no,sse=no) tool-use=no json-schema=no] | claude-messages[stream(request=yes,live=yes,sse=yes) tool-use=no json-schema=no] | openai-chat-completions[stream(request=yes,live=yes,sse=yes) tool-use=no json-schema=yes] | openai-responses[stream(request=yes,live=yes,sse=yes) tool-use=no json-schema=yes]
transport=https://api.openai.com/v1/responses(Post) headers=3 body={"model":"gpt-4.1","input":[...],"stream":true}
stream-events=4 tools=0 turn-count=1 terminal=completed
response-result={"ok":true,"source":"repl"}
model-error: none
response-result.pretty:
{
  "ok": true,
  "source": "repl"
}
```

session transcript 中，结构化结果也会被种成一条独立系统行：

```text
[t2:response-result] result={"ok":true,"source":"repl"}
```

### bridge output

`--bridge-once` / `--bridge-remote-once` 的 submitted turn 会先给 summary，再在有结构化结果时展开 `response result:`：

```text
bridge-turn: mode=local-repl transport=cli-bridge session=session-1 prompt=bridge detail response=resp-1 transcript=2 response-result=yes
response result:
{
  "ok": true,
  "source": "bridge-cli"
}
```

其中：

- `response-result=yes|no` 表示本次 bridge turn 是否携带 `response_result`
- wire JSON 字段固定为 `response_result`
- 旧桥接 payload 若仍写 `structured_output`，当前仍可被解码

### diagnostics 面对照

| 面 | 典型入口 | 主要用途 |
| --- | --- | --- |
| provider/status | `--status` | 看 provider、transport、capability matrix、error 摘要 |
| session diagnostics | REPL / TUI transcript 顶部 | 看当前 turn 的 transport、stream-events、terminal、`response-result.pretty` |
| bridge summary | `--bridge-once` / `--bridge-remote-once` | 看 bridge turn 是否带结果，以及 bridge wire 输出面 |
| task result 面 | `/tasks` / `/task-show` | 看 task 的 `result=`、`result.pretty:`、`result:agent` / `result-structured:*` 过滤 |

task 结果面示例：

```text
  a0000000000000002 type=agent status=completed queue=- summary=agent agent-a agent=agent-a tools=2 tokens=64 retrieved=true result={"kind":"agent","retrieved":true,"progress":{"tool_use_count":2,"token_count":64},"response_result":{"ok":true,"source":"task-panel"}}
```

## 环境变量

### Provider / model transport

| 变量 | 用途 |
| --- | --- |
| `OPENAI_API_KEY` | OpenAI 鉴权 token |
| `OPENAI_BASE_URL` | 覆盖 OpenAI base URL |
| `OPENAI_ORG_ID` | 可选 OpenAI organization header |
| `ANTHROPIC_API_KEY` | Anthropic 鉴权 token |
| `ANTHROPIC_BASE_URL` | 覆盖 Anthropic base URL |
| `NOCODE_PROVIDER_TIMEOUT_SECS` | provider HTTP timeout |
| `NOCODE_PROVIDER_RETRY_ATTEMPTS` | provider 总尝试次数，含首次请求 |
| `NOCODE_PROVIDER_RETRY_BACKOFF_MS` | retry 基础退避毫秒 |
| `NOCODE_PROVIDER_RETRY_MAX_BACKOFF_MS` | retry 最大退避毫秒 |

### Remote bridge transport

| 变量 | 用途 |
| --- | --- |
| `NOCODE_BRIDGE_BASE_URL` | 启用真实 HTTP remote bridge transport；未设置时 `--bridge-remote-once` 走 loopback |
| `NOCODE_BRIDGE_REQUEST_PATH` | request 路径，默认 `/v1/bridge/request` |
| `NOCODE_BRIDGE_PERMISSION_PATH` | permission callback 路径，默认 `/v1/bridge/permission` |
| `NOCODE_BRIDGE_RESPONSE_PATH` | response 路径，默认 `/v1/bridge/response` |
| `NOCODE_BRIDGE_TIMEOUT_SECS` | bridge HTTP timeout |
| `NOCODE_BRIDGE_AUTH_TOKEN` | Bearer token |
| `NOCODE_BRIDGE_AUTH_HEADER` | 自定义 auth header 名 |
| `NOCODE_BRIDGE_AUTH_VALUE` | 自定义 auth header 值 |

### Task agent host

| 变量 | 用途 |
| --- | --- |
| `NOCODE_TASK_AGENT_HOST` | 设为 `process`、`daemon` 或 `external` 时，将 local agent task 路由到外部 host |
| `NOCODE_TASK_AGENT_COMMAND` | 自定义外部 agent host 命令；设置后优先于内置 `--process-agent-host` |
| `NOCODE_TASK_AGENT_ARGS` | 外部 agent host 额外参数，按空格切分 |
| `NOCODE_TASK_AGENT_DAEMON_RESTARTS` | daemon 模式下单次请求允许的自动重启次数，默认 `1` |
| `NOCODE_TASK_AGENT_DAEMON_MAX_CONSECUTIVE_FAILURES` | 连续失败闸门；达到上限后即便还有 restart budget 也不再自动重启 |
| `NOCODE_TASK_AGENT_DAEMON_BACKOFF_STRATEGY` | 设置 default backoff profile 的 strategy，支持 `linear` 与 `exponential` |
| `NOCODE_TASK_AGENT_DAEMON_BACKOFF_JITTER_PERCENT` | 设置 default backoff profile 的 jitter 百分比 |
| `NOCODE_TASK_AGENT_DAEMON_BACKOFF_MS` | 设置 default backoff profile 的 base delay 毫秒 |
| `NOCODE_TASK_AGENT_DAEMON_IO_BACKOFF` | 覆盖 IO 类失败 profile，格式 `<base_ms>[:linear|exponential][:jitter_pct]` |
| `NOCODE_TASK_AGENT_DAEMON_DECODE_BACKOFF` | 覆盖 decode 类失败 profile，格式同上 |
| `NOCODE_TASK_AGENT_DAEMON_EXIT_BACKOFF` | 覆盖 process-exit 类失败 profile，格式同上 |
| `NOCODE_TASK_AGENT_DAEMON_RESTART_ON_IO_ERROR` | 是否在 pipe/write/read 类错误后自动重启 |
| `NOCODE_TASK_AGENT_DAEMON_RESTART_ON_DECODE_ERROR` | 是否在非法 JSON / 协议错位后自动重启 |
| `NOCODE_TASK_AGENT_DAEMON_RESTART_ON_CLEAN_EXIT` | 是否在 daemon `exit 0` 后仍自动拉起下一次请求 |

#### Backoff Profile 语义

- `BACKOFF_MS / STRATEGY / JITTER_PERCENT` 只定义 default profile。
- `IO / DECODE / EXIT` 若未显式设置，则回落到 default profile。
- 一旦设置 `*_IO_BACKOFF`、`*_DECODE_BACKOFF` 或 `*_EXIT_BACKOFF`，对应 failure class 使用该 profile 自己的 `base_ms / strategy / jitter_pct`。
- specific override 不继承 default profile 的 `strategy` 或 `jitter`。
- compact profile 允许省略后半段；省略时回落为该 profile 自身默认值：`linear + 0`。例如 `10::7` 等价于 `10:linear:7`。

## Demo 命令

### 本地 REPL / TUI

```bash
cd rust
cargo run -p nocode -- --repl
cargo run -p nocode -- --tui
```

### 外部 process agent host

```bash
cd rust
export NOCODE_TASK_AGENT_HOST=process
cargo run -p nocode -- --repl
```

此时 `/task-agent agent-a "rewrite provider adapter"` 会通过当前 `nocode` 可执行文件的 `--process-agent-host` 子进程执行。

若要复用常驻子进程：

```bash
cd rust
export NOCODE_TASK_AGENT_HOST=daemon
cargo run -p nocode -- --repl
```

若要观察 supervisor 策略：

```bash
cd rust
export NOCODE_TASK_AGENT_HOST=daemon
export NOCODE_TASK_AGENT_DAEMON_RESTARTS=2
export NOCODE_TASK_AGENT_DAEMON_MAX_CONSECUTIVE_FAILURES=2
export NOCODE_TASK_AGENT_DAEMON_BACKOFF_STRATEGY=exponential
export NOCODE_TASK_AGENT_DAEMON_BACKOFF_JITTER_PERCENT=15
export NOCODE_TASK_AGENT_DAEMON_BACKOFF_MS=50
export NOCODE_TASK_AGENT_DAEMON_DECODE_BACKOFF=100:exponential:0
export NOCODE_TASK_AGENT_DAEMON_IO_BACKOFF=25:linear:5
export NOCODE_TASK_AGENT_DAEMON_RESTART_ON_DECODE_ERROR=true
export NOCODE_TASK_AGENT_DAEMON_RESTART_ON_CLEAN_EXIT=false
cargo run -p nocode -- --repl
```

此配置下：

- default profile = `50ms + exponential + 15% jitter`
- decode profile = `100ms + exponential + 0% jitter`
- io profile = `25ms + linear + 5% jitter`
- 未显式设置 `EXIT_BACKOFF` 时，process-exit 回落到 default profile

`/runtime`、`/tasks` 与 TUI status line 会显示 `last_backoff_profile`，用于观测本次实际命中的 `default`、`io`、`decode` 或 `exit`。

### 本地 bridge / remote bridge

```bash
cd rust
cargo run -p nocode -- --bridge-once "bridge rewrite"
cargo run -p nocode -- --bridge-remote-once "remote bridge rewrite"
```

### 真实 HTTP remote bridge

```bash
cd rust
export NOCODE_BRIDGE_BASE_URL=http://127.0.0.1:8787
export NOCODE_BRIDGE_AUTH_TOKEN=dev-token
cargo run -p nocode -- --bridge-remote-once "remote bridge over http"
```

remote demo 会按顺序访问：

1. `/v1/bridge/request`
2. `/v1/bridge/permission`
3. `/v1/bridge/response`

bridge submitted wire 的结果字段统一为 `response_result`；旧 payload 若仍发送 `structured_output`，当前解码层会兼容接收。

## 上线 blocker 总览

若要把 `nocode` 当作真正可上线替代，至少还要补齐：

1. provider live streaming transport、capability matrix、清晰错误面。
2. task 持久恢复、取消清理、remote daemon/service。
3. concrete remote bridge service、session registry/resume、重连。
4. TUI permission/runtime/session 四条主链的持续交互。
5. packaging / CI / release / compat 成套闭环。

详细长 TODO 见 [DESIGN.md](/home/telagod/project/redcode/rust/DESIGN.md)。

## 外部 process agent host 协议

`ProcessTaskAgentHost` 当前支持两种最小协议：

- one-shot：`stdin JSON -> stdout JSON`
- daemon：`stdin NDJSON -> stdout NDJSON`

输入：

```json
{"agent_id":"daemon-a","prompt":"route prompt"}
```

输出：

```json
{"tool_use_delta":1,"token_delta":42,"retrieved":true,"status":"completed","response_result":{"ok":true}}
```

兼容说明：

- 新协议字段：`response_result`
- 旧协议字段：`structured_output`
- 当前 runtime 对旧字段只保留 decode 兼容；新输出一律应发 `response_result`

当前单测已用 `python3 -c` 跑通此协议，可对照 [task_runtime.rs](/home/telagod/project/redcode/rust/crates/nocode-core/src/task_runtime.rs)。

## 目录结构

```text
rust/
├── Cargo.toml
├── DESIGN.md
├── README.md
└── crates/
    ├── nocode/
    │   └── src/
    │       ├── main.rs
    │       ├── repl.rs
    │       ├── task_panel.rs
    │       └── tui.rs
    └── nocode-core/
        └── src/
            ├── bridge_runtime.rs
            ├── provider.rs
            ├── provider_transport.rs
            ├── query_engine.rs
            ├── query_loop.rs
            ├── roadmap.rs
            ├── session_persistence.rs
            ├── task_runtime.rs
            ├── tool_execution.rs
            └── ...
```
