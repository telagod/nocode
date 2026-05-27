# 07 · Release & development

> Last updated: 2026-05-27 · Owner: tooling · Read with [02_architecture](./02_architecture.md), [08_roadmap](./08_roadmap.md)

How nocode gets from `main` to your `~/.local/bin`. And how to develop on it without breaking the contracts.

## Versioning

[Semver](https://semver.org/), but we're 0.x. That means:

- **Minor bumps** (`0.3 → 0.4`) can break public surface. Each one ships with an actionable migration message at runtime *and* a CHANGELOG section.
- **Patch bumps** (`0.3.0 → 0.3.1`) never break public surface. New behaviour is additive, behind a flag, or under an env var.

Public surface, for the purpose of versioning, is:

- The CLI flags listed by `nocode --help`.
- The TOML schema documented in [10_provider_config.md](./10_provider_config.md).
- The 11 default tools and their JSON schemas (`tool_registry_has_canonical_core_set` asserts this).
- The `Provider` trait + `ResolvedProvider` struct (for downstream embedders).
- The npm package layout (main `@telagod/nocode` + 6 platform packages).

Everything else — internal modules, sub-tests, telemetry shapes — is free to move.

## CI workflows

Three workflows in `.github/workflows/`:

| File | Trigger | Purpose |
|---|---|---|
| `ci.yml` | push / PR to `main` | `cargo fmt --check` → `clippy -D warnings` → `cargo test` |
| `release.yml` | tag `v*` | 6-platform build → GitHub Release → npm publish (7 packages) |
| `pages.yml` | push to `main` touching `docs/landing/**` | Deploy `docs/landing/index.html` to GitHub Pages |

The CI gate is hard: **fmt + clippy strict + every test green** on every PR. There is no `[skip ci]` allowed in commit messages.

## Cutting a release

The whole process is **two commits and a tag**:

```bash
# 1. Land everything you want in the release on main with green CI.
# 2. Bump version in workspace Cargo.toml:
sed -i 's/^version = "0.3.0"/version = "0.3.1"/' Cargo.toml
cargo build      # let Cargo.lock catch up

# 3. Write the CHANGELOG entry — see "Changelog discipline" below.
# 4. Commit the bump + changelog:
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "chore(release): v0.3.1"
git push origin main

# 5. After CI is green, tag and push:
git tag v0.3.1 -m "v0.3.1 — <one-line summary>"
git push origin v0.3.1
```

The tag push triggers `release.yml`, which:

1. Builds 6 platform binaries (linux x64/arm64, macOS x64/arm64, Windows x64/arm64).
2. Creates the GitHub Release with all 6 binaries attached.
3. Publishes 7 npm packages with the same version — main `@telagod/nocode` + 6 platform packages — via `npm publish --access public --provenance` with Sigstore attestation.

`release.yml` **fails loud on non-409 npm errors** (added after v0.3.0 was bitten by a silent-skip + expired token). The only error swallowed is "version already published" (E409 / EPUBLISHCONFLICT) so a tag re-push is a safe no-op for already-published platform packages.

## NPM token rotation

The `NPM_TOKEN` secret is a [Granular Access Token](https://docs.npmjs.com/about-access-tokens) scoped to `@telagod` with `Publish` permission. Defaults to **30-day expiry**.

When CI starts failing with `npm error 404` on PUT to `/registry.npmjs.org/...` (which `release.yml` will now surface loudly), the token has expired. Rotation:

1. https://www.npmjs.com/ → Access Tokens → Generate New Token
2. Type: **Granular** · Scope: `@telagod` · Permission: **Publish** · Expiry: 1 year
3. Repo → Settings → Secrets and variables → Actions → update `NPM_TOKEN`
4. Re-run the failed release: `git push --delete origin v0.x.x && git push origin v0.x.x`

## Changelog discipline

`CHANGELOG.md` is the contract with downstream. Per release:

- **Highlights** (3–5 bullets) — what a user reading the release announcement should know.
- **Added** — every new public surface element. Group by area if more than 10.
- **Changed** — anything that looks the same but behaves differently.
- **Removed (breaking)** — the migration block. Always include a `before`/`after` diff if config or CLI changed.
- **Fixed** — only user-visible bugs. Internal refactors don't go here.
- **Stats** — tests count, clippy status, net LOC vs the previous tag. This makes the shape of the release legible.

Patch bumps may omit any subsection that's empty.

## Development

### Build matrix

```bash
cargo build                                                       # debug
cargo build --release                                             # release
cargo build --no-default-features -p nocode-core --features minimal
                                                                  # minimal core
                                                                  # (no MCP, no plugins, no telemetry, no OAuth)
```

### Test matrix

```bash
cargo test                                       # full sweep, ~5s
cargo test -p nocode-core                        # core library only
cargo test -p nocode                             # TUI/CLI binary only
cargo test -p nocode-core --test roadmap         # contract tests
cargo test -p nocode-core --test tool_roundtrip  # executor pipeline
cargo test -p nocode-core --test trust_mcp       # trust + permission + MCP
cargo test <test_name>                           # single test by name
```

Every test that mutates process-global state (`$HOME`, `cwd`, `NOCODE_*` env) must lock `crate::test_support::env_mutex()` to serialize. See `tool::skill::tests` for the pattern.

### Lint + format

```bash
cargo clippy --all-targets --no-deps -- -D warnings    # what CI runs
cargo fmt --check                                       # what CI checks
cargo fmt                                               # autofix
```

`-D warnings` is non-negotiable. `clippy::pedantic` and `clippy::nursery` are enabled at warn-level; if a rule is too noisy in a specific spot, gate it with `#[allow(clippy::...)]` and a one-line comment explaining why.

### Adding a new tool

1. New file under `crates/nocode-core/src/tool/` implementing the `Tool` trait (`name`, `description`, `input_schema`, `execute`).
2. Add the `pub mod` line to `crates/nocode-core/src/tool/mod.rs`.
3. If it's an opt-in tool — **stop here**. Document it in [02_architecture.md](./02_architecture.md). Users register it explicitly.
4. If it really must be in the default registry — and only if it earns its keep with one job and zero overlap — then:
   - Add it to `ToolRegistry::with_defaults`.
   - Add its name to `ToolRegistry::core_tool_names()`.
   - The `tool_registry_has_canonical_core_set` test will pass automatically.
   - Bump to a **minor** version (breaking the size of the default registry is a public-surface change).

### Adding a new provider wire format

1. Implement the `Provider` trait in `crates/nocode-core/src/provider/<name>.rs`.
2. Add the string to `normalize_api_format()` accepted set in `config/settings.rs`.
3. Add the `is_known_wire_api()` arm in `provider/resolve.rs`.
4. Wire it through `main.rs::build_provider()` and `tool/agent.rs::build_worker_provider()`.
5. Add a `resolve_named_provider` integration test covering the new wire string.
6. Document it in [10_provider_config.md](./10_provider_config.md).

### Skill conventions

Skills live in `.nocode/skills/` and `.claude/skills/`. The discovery + parsing rules are in [03_skills.md](./03_skills.md). If you're adding a skill that should ship with the binary by default, that's a *bundled* skill — talk about it in [08_roadmap](./08_roadmap.md) first; bundling is a public-surface decision.
