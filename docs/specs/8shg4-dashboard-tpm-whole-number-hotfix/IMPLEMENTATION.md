# Dashboard TPM 整数显示热修（#8shg4) - Implementation

## Current State

- Canonical spec: `docs/specs/8shg4-dashboard-tpm-whole-number-hotfix/SPEC.md`
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-04-11
- Last: 2026-04-11

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: `TodayStatsOverview.test.tsx` 覆盖 fractional TPM 整数显示。
- Integration tests: `DashboardActivityOverview.test.tsx` 保持 today rate flow 回归不破。

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增 follow-up spec 索引并同步状态。
- `docs/specs/8shg4-dashboard-tpm-whole-number-hotfix/SPEC.md`: 记录本次格式热修的范围、验收与验证。
