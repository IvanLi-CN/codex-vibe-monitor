# Bun-first 工具链收敛 - Implementation

## Current State

- Canonical spec: `docs/specs/tr4ev-bun-first-toolchain/SPEC.md`
- Implementation summary: 已完成
- Current toolchain baseline: Bun `1.3.14` / Rust `1.96.0` / GitHub-hosted x64 `ubuntu-24.04` / release arm64 `ubuntu-24.04-arm`

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
- `bash .github/scripts/test-quality-gates-contract.sh`
- `bash .github/scripts/test-build-smoke-image-with-retry.sh`
- `bash .github/scripts/test-release-snapshot.sh`
- `cd web && bun run lint`
- `cd web && bun run test`
- `cd web && bun run build`
- `cd web && bun run storybook:build -- --quiet`
- `bun run check:bun-first`
- `docker build -t codex-vibe-monitor:gha-env-refresh --build-arg APP_EFFECTIVE_VERSION=dev .`

## Migrated Implementation Sections

### Quality checks

- `codex --sandbox read-only -a never review --base origin/main`
- PR required checks 保持为 `Validate PR labels`、`Lint & Format Check`、`Backend Tests`、`Build Artifacts`、`Review Policy Gate`
- quality-gates contract fixtures、release snapshot helper、Docker smoke retry helper 与 live workflows 的 runner / action ref 基线保持一致

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 建立 Bun-first spec，冻结“只改直接执行面、不动业务接口”的范围。
- [x] M2: 完成仓库根与 `web/` 的 Bun lockfile、脚本、hooks、Docker、CI、文档迁移。
- [x] M3: 跑通本地验证、Docker smoke、PR checks 与 review-loop 收敛。
