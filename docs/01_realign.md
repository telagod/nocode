# nocode REALIGN — harness 工程仿生学定位

> Created: 2026-05-26 · Status: proposal · Supersedes parity goals in DESIGN.md

## 一、判词

**nocode 当前的病灶不在能跑不能跑，在「把自己钉死在 Claude Code parity 上」。**

证据链：

| 维度 | 当前状态 | 引用 |
|---|---|---|
| 工具数 | **28 个** `impl Tool for ...` | `crates/nocode-core/src/tool/` 28 个 `impl` |
| 门禁层 | **6 层** + bash 子门禁 6 个 | `executor.rs:18-22`, `bash_validation.rs:3-9` |
| 单文件 LOC | executor 25.8KB, permission 21.5KB, bash_validation 24.8KB | 同上 |
| Skill 地位 | wrapper 工具，返回 prompt 字符串 | `tool/skill.rs` 仅 6KB，**未进入 prompt assembly** |
| 自我定位 | "Rust reimplementation of Claude Code's execution kernel" + "21 tools strict parity" | `docs/DESIGN.md:6-9` |

结论：parity 是工程惯性，不是产品定位。要换骨。

---

## 二、对照系（已实地查过的项目）

### oh-pi (`~/project/oh-pi`)
- 实际是 [pi-coding-agent](https://github.com/badlogic/pi-mono) 的 **bootstrap/distribution package**，TypeScript 写的 TUI wizard
- 哲学：**配置 wizard → preset → 多 agent colony** —— 不是 harness 设计参考
- **commit 10eb7b6 "Pi-inspired" 指的是 Inflection AI Pi 聊天 UI**（无符号、留白、字色作层级），不是 oh-pi 本身

### codex (`which codex` → fnm 全局；`~/.code-abyss/.../codex/SKILL.md` 有模板)
- 关键文物：**`SKILL.md.tmpl`** —— codex 把 skill 当工程契约，含 mode detection / filesystem boundary / 两阶段流程
- 公开仓库 `github.com/openai/codex` 是 Rust + npm 双发，**ApprovalPolicy + SandboxPolicy** 两层而非 nocode 6 层
- nocode 该偷的：**skill 作为可执行契约**而不是"读个 md 返回字符串"

### nullclaw / claw-code (`~/project/nullclaw`)
- Zig，~1MB RAM，<8ms 启动，35+ tools，10 memory engines
- 哲学："Null overhead. Null compromise."
- 该偷的：**lean by default**, 静态契约, sandbox 多策略可选（landlock/firejail/bubblewrap/docker）

---

## 三、定位（一句话）

> **nocode = 分形智能 code agent 的最小 harness 参考实现。**
> 工具是骨，skill 是肉，门禁是皮。骨要少，肉要厚，皮要薄。

四条铁律：

1. **Skill 是一等公民**（不是工具）—— 进入 prompt assembly，与 CLAUDE.md / AGENTS.md 同级
2. **最少工具**：12 个原子工具，跨度 ≤ 1（一个工具做一件事，不重叠）
3. **最少硬门禁**：3 层（schema → policy → hook），其余作为 informational signal
4. **分形**：sub-agent 共享主 loop 同一份 Provider+ToolRegistry+SkillRegistry+PermissionMode，递归即分形

---

## 四、斩链 · 路线图

### Phase 1 — 骨架对齐（破坏性中，可逆）

#### 1.1 工具瘦身 28 → 12

| 保留（12） | 撤为 slash 命令（7） | 删除/合并（9） |
|---|---|---|
| Read, Write, Edit | EnterPlanMode/ExitPlanMode → `/plan` | NotebookEdit（合到 Edit） |
| Bash | EnterWorktree/ExitWorktree → `/wt` | CronCreate/Delete/List（移到 plugin） |
| Glob, Grep | TeamCreate/TeamDelete → `/team` | ConfigTool（合到 slash `/config`） |
| WebFetch, WebSearch | | SendMessage（合到 Agent） |
| Task, Agent | | LspTool（移到 plugin） |
| AskUserQuestion | | |
| **Skill** | | ToolSearchTool（核心化为内部 dispatcher） |
| MemoryTool, McpTool | | |
| TodoWrite | | |

#### 1.2 门禁折叠 6 → 3

```
现在: validation → trust → hooks(pre) → permission → sandbox → execute → hooks(post)
变成: schema → policy(trust+permission+sandbox 合一) → hooks → execute → hooks(post)
```

`bash_validation.rs` 6 子模块 → 单个 `BashClassifier::classify() -> ToolRisk { Safe | Mutating | Destructive }`，让 policy 层基于 risk + permission_mode 做决策。**单文件从 24.8KB → 目标 8KB**

#### 1.3 Skill 升格

```rust
// 新增：crates/nocode-core/src/skill/mod.rs
pub struct SkillRegistry { skills: BTreeMap<String, SkillDef> }
pub struct SkillDef { name, description, frontmatter, body, search_path }
```

- `prompt/assembly.rs` 增加第 4 个 block：`available_skills` 列表（仅 name + description，<2KB）
- 用户问到匹配的关键词时，模型可主动 `Skill(name=...)` 调用
- 现有 `tool/skill.rs` 重构为 `SkillRegistry::invoke()` 的薄壳

### Phase 2 — 可解释（破坏性低）

- TUI 已经 Pi-inspired（10eb7b6 方向对），下一步：
  - **Why-trail**：每次工具调用展示 `policy decision → reason`（不只是 allow/deny）
  - **Skill match indicator**：显示模型为什么选了某个 skill（topic match / explicit invoke）
  - **Budget visibility**：当前已在 input/output token 显示，扩展为分形预算树（sub-agent 占了多少）

### Phase 3 — 分形（破坏性中）

- `AgentTool` 重构为递归：sub-agent = 完整 `QueryEngine`，共享 deps 但独立 budget
- Sub-agent 可继承父 SkillRegistry 子集，也可加载自己的 `.nocode/skills/sub-agent-name/`
- 输出嵌套折叠 TUI 渲染

### Phase 4 — DESIGN.md 改名为 PHILOSOPHY.md

把 "21 tools strict parity" 改为 "12 atomic tools + skill-loaded fractal"。同时归档 ALIGNMENT.md（已过期）。

---

## 五、验尸（每 phase 的验证）

| Phase | 验证 |
|---|---|
| 1 | `cargo test` 全绿；`grep -c 'impl Tool for' = 12`；bash_validation.rs < 10KB |
| 2 | TUI 跑一个 Edit + Skill 调用，截图显示 why-trail |
| 3 | sub-agent 跑 nested task，budget 树状显示，无 share-state bug |
| 4 | 文档 grep "parity" 为空 |

---

## 六、余劫

- 工具撤回 slash 后，**MCP/plugin 暴露的工具发现要 backward-compatible**（外部脚本可能依赖 EnterPlanMode 工具名）
- bash_validation 合并风险：现有 619+ 行的测试要全部保住
- Skill 升格进 prompt 后 token 预算要重算（每个 skill 描述 ~50 tokens，10 个 skill ~500 tokens）

---

## 七、再斩

一个 PR 一个 Phase。**Phase 1 = 最贵也最值**。建议先做 1.3（Skill 升格）—— 这一步独立可验证，且能立即让 nocode 区别于 Claude Code parity。

魔尊点头则起手。
