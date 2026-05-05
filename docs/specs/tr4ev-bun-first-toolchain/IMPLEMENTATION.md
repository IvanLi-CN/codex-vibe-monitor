# Bun-first 工具链收敛 - Implementation

## Current State

- Canonical spec: `docs/specs/tr4ev-bun-first-toolchain/SPEC.md`
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-03-12
- Last: 2026-03-12

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `bun install --frozen-lockfile`
- `cd web && bun install --frozen-lockfile`
- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `cargo test --locked --all-features`
- `cd web && bun run lint`
- `cd web && bun run test`
- `cd web && bun run build`
- `cd web && bun run build-storybook`
- `bun run check:bun-first`
- `docker build -t codex-vibe-monitor:bun-smoke --build-arg APP_EFFECTIVE_VERSION=dev .`
- `./.github/scripts/smoke-test-image.sh codex-vibe-monitor:bun-smoke`

## Migrated Implementation Sections

### Quality checks

- `codex --sandbox read-only -a never review --base origin/main`
- PR required checks 保持为 `Validate PR labels`、`Lint & Format Check`、`Backend Tests`、`Build Artifacts`、`Review Policy Gate`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 建立 Bun-first spec，冻结“只改直接执行面、不动业务接口”的范围。
- [x] M2: 完成仓库根与 `web/` 的 Bun lockfile、脚本、hooks、Docker、CI、文档迁移。
- [x] M3: 跑通本地验证、Docker smoke、PR checks 与 review-loop 收敛。
