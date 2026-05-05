# 接入 Claude Relay 日统计源 - Implementation

## Current State

- Canonical spec: `docs/specs/vz7cz-claude-relay-api-stats/SPEC.md`
- Migrated from legacy source: `docs/plan/0001:claude-relay-api-stats/PLAN.md`
- Legacy source retention: pending delete approval
- Implementation summary: 待实现

## Migrated Implementation Notes

## 状态

- Status: 待实现
- Created: 2026-01-16
- Last: 2026-01-16

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: 外部统计增量计算、回退/日切换处理。
- Integration tests: 统计聚合（含多来源合并）与 API 返回一致性。
- E2E tests (if applicable): 暂无。

## 文档更新（Docs to Update）

- `README.md`: 新增外部统计源配置项与合并口径说明。
- `docs/system-design.md`: 更新“数据来源定位/入库设计/统计口径”章节。
