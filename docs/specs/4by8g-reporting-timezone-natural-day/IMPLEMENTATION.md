# 统计按浏览器时区自然日 - Implementation

## Current State

- Canonical spec: `docs/specs/4by8g-reporting-timezone-natural-day/SPEC.md`
- Migrated from legacy source: `docs/plan/0004:reporting-timezone-natural-day/PLAN.md`
- Legacy source retention: pending delete approval
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-02-20
- Last: 2026-02-20

## 里程碑（Milestones）

- [x] M1: 后端引入 `timeZone` 参数并修复 `occurred_at` 下界绑定。
- [x] M2: 实现 `bucket=1d` 的严格自然日分桶与 DST 覆盖。
- [x] M3: 前端默认附带浏览器时区，并修复日历视图的 off-by-one 与展示口径提示。
