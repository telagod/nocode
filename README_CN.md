<div align="center">

# nocode

**分形 code agent 最小化的 harness。**

基于 Rust 的终端 AI 编程代理 —— 11 个原子工具，3 道可解释门禁，技能作为一等公民的 prompt 资源，子 agent 与父 agent 共享同一份 harness。

[![npm](https://img.shields.io/npm/v/@telagod/nocode?label=npm&color=a78bfa)](https://www.npmjs.com/package/@telagod/nocode)
[![tests](https://img.shields.io/badge/tests-825%20green-86efac)](#%E7%8A%B6%E6%80%81)
[![clippy](https://img.shields.io/badge/clippy-D%20warnings%20clean-86efac)](https://github.com/telagod/nocode/blob/main/.github/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/telagod/nocode?color=a78bfa)](https://github.com/telagod/nocode/releases/latest)
[![license](https://img.shields.io/npm/l/@telagod/nocode?color=6e6a7d)](LICENSE)

[**官网**](https://telagod.github.io/nocode/) ·
[English](README.md) ·
[CHANGELOG](CHANGELOG.md) ·
[文档](docs/README.md)

</div>

---

> **v0.3.0 不兼容版本。** 旧的 `custom_*` 配置方案和 `--login` 向导已删除，改为 codex 风格的命名 provider 表 + `nocode init` / `nocode config`。从 0.2.x 升级？直接运行 `nocode` —— 启动报错会用精确 diff 指引你迁移。或读 [CHANGELOG](CHANGELOG.md)。

## 30 秒上手

```bash
npm install -g @telagod/nocode
nocode init                           # 生成 ~/.nocode/config.toml
export OPENAI_API_KEY=sk-...          # 或对应 provider 的 key
nocode                                # 启动 TUI
```

只有一个 API key 一个端点？跳过配置文件：

```bash
export ANTHROPIC_API_KEY=sk-ant-...
nocode --provider claude              # 内置别名，开箱即用
```

内置别名：`claude` · `openai` · `gemini`。

## 四条不变量

整个产品就是这四条，其余一切都是推论。

1. **技能一等公民** —— `.nocode/skills/` 下的 `*.md` 文件与 `CLAUDE.md` 同级注入 prompt。模型在动作*之前*就看到全部索引，正文按需加载。→ [docs/03_skills.md](docs/03_skills.md)
2. **11 个原子工具** —— 一工具一职，零重叠。默认注册表由集成测试锁死，不会悄悄膨胀。其余工具按需启用。→ [docs/02_architecture.md](docs/02_architecture.md)
3. **三道可解释门禁** —— `Schema → Policy → Hooks`。每次拒绝都带 `Denied [<门>: <原因>]`。零静默拒绝。→ [docs/04_policy_gates.md](docs/04_policy_gates.md)
4. **分形子 agent** —— `Agent` 派生出来的子 agent 运行*同一份* harness，递归同时继承能力和约束。→ [docs/05_fractal_subagents.md](docs/05_fractal_subagents.md)

## 11 个核心工具

| | | |
|---|---|---|
| `FileRead` 读取文件 | `FileWrite` 新建 | `FileEdit` 编辑 |
| `Glob` 路径搜索 | `Grep` 内容搜索 | `Bash` 执行 |
| `WebFetch` 抓取 URL | `WebSearch` 搜索 | `Agent` 派生子 agent |
| `AskUserQuestion` 询问 | `Skill` 调用技能 | |

可选工具（`Memory`、`TodoWrite`、`Cron*`、`Team*`、`Mcp`、`NotebookEdit`、`Lsp` 等）独立模块，按需启用。

## 配置

一个 TOML 文件。`nocode init` 一次生成带注释模板。

```toml
# ~/.nocode/config.toml
default_provider = "subfox"
model = "gpt-5.5"
permission_mode = "ask"

[providers.subfox]
base_url    = "https://sub.foxnio.com/v1"
wire_api    = "openai-responses"      # anthropic | openai-responses | openai-chat | google
api_key_env = "OPENAI_API_KEY"        # 持有 key 的环境变量名 —— 显式声明，无回退链
default_model = "gpt-5.5"

[providers.local-vllm]
base_url    = "http://localhost:8000/v1"
wire_api    = "openai-chat"
api_key_env = "VLLM_API_KEY"

[profiles.work]
provider = "subfox"

[profiles.home]
provider = "local-vllm"
permission_mode = "auto"
```

一行切换：

```bash
nocode --provider local-vllm     # 临时切换
nocode --profile work            # 应用一个 profile
NOCODE_PROVIDER=openai nocode    # 通过环境变量
```

完整 schema、优先级链与迁移指引：**[docs/10_provider_config.md](docs/10_provider_config.md)**。

## CLI 一览

```bash
nocode                              # 交互式 TUI（默认）
nocode init [--force]               # 生成 ~/.nocode/config.toml
nocode config <list|get|set|unset>  # 查看 / 修改标量配置
nocode --status                     # 诊断 + 当前 sqlite 卷 + 高频工具与拒绝门
nocode insight [where|sessions|tools|gates|cost]  # 观察 —— 提问，而非仪表盘
nocode --provider <name>            # 临时切 provider
nocode --profile <name>             # 临时切 profile
nocode --resume [<session-id>]      # 恢复历史会话（-c 简写）
nocode --bridge-once "<prompt>"     # 单轮非交互执行
nocode --help                       # 完整 flag 帮助
```

## TUI 按键

| 按键 | 行为 |
|---|---|
| `↑` / `↓` · `PgUp` / `PgDn` | 滚动 |
| `Ctrl-P` / `Ctrl-N` | 输入历史 |
| `Ctrl-U` | 清空输入 |
| `Ctrl-O` | 展开 / 折叠工具输出或 thinking |
| `Esc` | 取消当前流 / 关闭浮层 |
| `F1` / `?` | 帮助浮层 |

## 环境变量

| 变量 | 用途 |
|---|---|
| `NOCODE_PROVIDER` | `[providers.<name>]` 表里的名字（或内置别名） |
| `NOCODE_PROFILE` | `[profiles.<name>]` 表里的名字 |
| `NOCODE_MODEL` | 覆盖模型 |
| `NOCODE_SYSTEM_PROMPT` | 覆盖系统 prompt |
| `NOCODE_MODEL_REASONING_EFFORT` | `low`、`medium`、`high` |
| `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `GEMINI_API_KEY` | provider API key |
| `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` | 单 provider base URL 覆盖 |

## 状态

| | |
|---|---|
| 最新版本 | **v0.3.0**（[CHANGELOG](CHANGELOG.md)） |
| 测试 | **825 个全绿**（160 TUI · 616 lib · 12 mock · 11 roadmap · 16 tool_roundtrip · 10 trust_mcp） |
| clippy | `--all-targets -- -D warnings` 干净 |
| 版本 | Rust 2024 · MSRV 1.85 |
| 平台 | linux / macOS / Windows · x64 + arm64（6 个二进制，7 个 npm 包） |

## 从源码构建

```bash
git clone https://github.com/telagod/nocode.git
cd nocode
cargo build --release
cp target/release/nocode ~/.local/bin/
```

两 crate workspace：

- `crates/nocode-core/` —— 库（约 28K LOC），全部核心逻辑
- `crates/nocode/` —— TUI / CLI 壳

```bash
cargo test                            # 全测试套
cargo clippy --all-targets -- -D warnings
cargo build --no-default-features -p nocode-core --features minimal  # 最小核心
```

## 文档

文档拆成编号系列 —— 短、单主题、可演进。**入口：[docs/README.md](docs/README.md)**。

| | |
|---|---|
| [00_vision.md](docs/00_vision.md) | nocode 存在的理由、四条不变量、闭环图 |
| [01_realign.md](docs/01_realign.md) | 2026/05 重定位 PRD |
| [02_architecture.md](docs/02_architecture.md) | 模块图、provider / loop / storage 布局 |
| [03_skills.md](docs/03_skills.md) | 一等公民技能模型 |
| [04_policy_gates.md](docs/04_policy_gates.md) | 三道门禁 + why-trail |
| [05_fractal_subagents.md](docs/05_fractal_subagents.md) | 子 agent 继承机制 |
| [06_observer.md](docs/06_observer.md) | 观察哲学 |
| [07_release.md](docs/07_release.md) | 发版流程与 CI |
| [08_roadmap.md](docs/08_roadmap.md) | 下一步 |
| [10_provider_config.md](docs/10_provider_config.md) | provider 表 + `nocode init` / `config` |

## 许可

MIT —— 见 [LICENSE](LICENSE)。

<div align="center">

<sub>用心打磨，不靠堆砌 · v0.3.0</sub>

</div>
