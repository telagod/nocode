# @telagod/nocode

Terminal-native AI coding assistant built in Rust. Connects to Claude, OpenAI, Gemini, or any compatible endpoint.

## Install

```bash
npm install -g @telagod/nocode
```

## Features

- Interactive REPL and 4-pane TUI with Markdown rendering and syntax highlighting
- 25 built-in tools (Read, Edit, Write, Bash, Glob, Grep, WebFetch, Agent, Tasks, Teams, MCP, LSP, Memory...)
- Multi-provider support: Claude, OpenAI, Gemini, or custom endpoints
- Session compaction with structured summaries
- SQL-backed storage with date-based volume partitioning
- Persistent memory system with auto-detection signals
- MCP client (JSON-RPC over stdio) for tool extensibility
- Plugin system with lifecycle management

## Usage

```bash
nocode --repl          # interactive REPL
nocode --tui           # 4-pane terminal UI
nocode --status        # system diagnostics
```

## Configuration

Set your provider API key:

```bash
export ANTHROPIC_API_KEY=sk-...    # Claude
export OPENAI_API_KEY=sk-...       # OpenAI
export GEMINI_API_KEY=...          # Gemini
```

Override provider or model:

```bash
export NOCODE_MODEL_PROVIDER=claude   # claude|openai|gemini|custom
export NOCODE_MODEL=claude-opus-4-6
```

## Supported Platforms

| Platform | Package |
|----------|---------|
| Linux x64 | `@telagod/nocode-linux-x64` |
| macOS x64 | `@telagod/nocode-darwin-x64` |
| macOS ARM64 | `@telagod/nocode-darwin-arm64` |
| Windows x64 | `@telagod/nocode-win32-x64` |

## License

MIT
