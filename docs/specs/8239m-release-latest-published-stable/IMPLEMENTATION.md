# Release `latest` 仅指向最新已发布 stable - Implementation

## Current State

- Canonical spec: `docs/specs/8239m-release-latest-published-stable/SPEC.md`
- Implementation summary: 已实现，待 PR / CI 收敛

## Migrated Implementation Notes

## 状态

- Status: 已实现，待 PR / CI 收敛
- Created: 2026-03-29
- Last: 2026-03-29

## 非功能性验收 / 质量门槛（Quality Gates）

- `bash .github/scripts/test-release-snapshot.sh`

## Migrated Implementation Sections

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 拆分 immutable tags 与 publish-time tags 责任边界
- [x] M2: 让 `latest` 只受更高已发布 stable 影响
- [x] M3: README 与脚本级回归测试对齐新语义
- [ ] M4: fast-track 推进到 PR / CI / review 收敛
