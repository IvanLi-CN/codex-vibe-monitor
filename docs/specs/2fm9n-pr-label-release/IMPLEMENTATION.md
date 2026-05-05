# PR 标签驱动发版 - Implementation

## Current State

- Canonical spec: `docs/specs/2fm9n-pr-label-release/SPEC.md`
- Migrated from legacy source: `docs/plan/0002:pr-label-release/PLAN.md`
- Legacy source retention: pending delete approval
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-02-19
- Last: 2026-02-19

## 非功能性验收 / 质量门槛（Quality Gates）

- 本地至少执行 1 条与改动相关的自动化验证：
  - `bash -n .github/scripts/compute-version.sh`
- PR 的 CI（lint/unit-tests/build）应保持通过；label gate 应通过。

## 里程碑（Milestones）

- [x] M1: 创建 labels + 新增 label gate workflow
- [x] M2: release-intent gate + 版本计算（stable/rc）落地
- [x] M3: tag 创建幂等 + GitHub Release 创建幂等 + README 更新
