# Bun-first 工具链收敛 - Implementation

## Current State

- Canonical spec: `docs/specs/tr4ev-bun-first-toolchain/SPEC.md`
- Implementation summary: 已完成
- Current toolchain baseline: Bun `1.3.14` / Rust `1.96.0` / GitHub-hosted x64 `ubuntu-24.04` / release arm64 `ubuntu-24.04-arm`
- Static analysis baseline: root Biome `2.5.3` for `web/` and `docs-site/`; Clippy runs with `-D warnings` in local hooks and the existing `Lint & Format Check` jobs.

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-03-12
- Last: 2026-06-23

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `bun install --frozen-lockfile`
- `bun install --cwd docs-site --frozen-lockfile`
- `cd web && bun install --frozen-lockfile`
- `cargo check --locked --all-targets --all-features`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- `bash .github/scripts/test-quality-gates-contract.sh`
- `bash .github/scripts/test-build-smoke-image-with-retry.sh`
- `bash .github/scripts/test-release-snapshot.sh`
- `bun run lint:web`
- `bun run lint:docs`
- `cargo fmt --all -- --check`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- `cd web && bun run test`
- `cd web && bun run build`
- `cd web && bun run storybook:build -- --quiet`
- `bun run check:bun-first`
- `docker build -t codex-vibe-monitor:gha-env-refresh --build-arg APP_EFFECTIVE_VERSION=dev .`

### Biome and Clippy quality-gate convergence

- Root `biome.json` owns JavaScript, TypeScript, JSON, and CSS checks for `web/` and `docs-site/`; Markdown remains under dprint.
- `web/public/mockServiceWorker.js` is excluded as generated code. Tailwind directives are parsed as CSS rather than globally suppressing unknown at-rule diagnostics.
- Biome recommended rules remain enabled. Existing behavior-sensitive React dependency, accessibility, key, and assertion diagnostics stay visible as warnings while parse, format, import organization, and all non-migrated diagnostics remain blocking; legacy Hook exceptions use file-level, reasoned Biome suppressions instead of ESLint comments.
- `web/` delegates `lint` to `bun run lint:web`; `docs-site/` delegates to `bun run lint:docs`; ESLint and its configuration have been removed.
- CI job names and required-check inventory remain unchanged while the existing lint job now installs Clippy and runs the strict command above.
- Visual verification uses `Dashboard/WorkingConversationsSection` `Wide Desktop 1660` Storybook canvas, with existing story interaction coverage retained.

## Visual Evidence

- Storybook canvas: `Dashboard/WorkingConversationsSection` / `Wide Desktop 1660`, captured from the deterministic local Storybook surface on 2026-07-12. The rendered current, placeholder, success, and failure conversation states retain their expected layout after the accessibility corrections.

## Migrated Implementation Sections

### Quality checks

- `codex --sandbox read-only -a never review --base origin/main`
- PR required checks 当前为 `Validate PR labels`、`Lint & Format Check`、`Front-end Tests`、`Records Overlay E2E`、`Backend Tests (Lightweight)`、`Backend Tests (Stateful SQLite)`、`Backend Tests (Archive / File I/O)`、`Build Artifacts`、`Review Policy Gate`
- quality-gates contract fixtures、release snapshot helper、Docker smoke retry helper 与 live workflows 的 runner / action ref 基线保持一致

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 建立 Bun-first spec，冻结“只改直接执行面、不动业务接口”的范围。
- [x] M2: 完成仓库根与 `web/` 的 Bun lockfile、脚本、hooks、Docker、CI、文档迁移。
- [x] M3: 跑通本地验证、Docker smoke、PR checks 与 review-loop 收敛。
