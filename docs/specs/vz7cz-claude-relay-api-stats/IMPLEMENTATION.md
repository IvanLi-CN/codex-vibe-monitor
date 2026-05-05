# 接入 Claude Relay 日统计源 - Implementation

## Current State

- Canonical spec: `docs/specs/vz7cz-claude-relay-api-stats/SPEC.md`
- Migrated from legacy source: `docs/plan/0001:claude-relay-api-stats/PLAN.md`
- Legacy source retention: pending delete approval
- Implementation summary: 已完成；legacy plan migration records this topic as archived.

## Migrated Implementation Notes

## 状态

- Status: 已完成
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

## Additional Migrated Task Notes

### UI / Storybook (if applicable)

- Stories to add/update: 暂无。
- Visual regression baseline changes (if any): 无。

### Quality checks

- `cargo fmt`
- `cargo check`
- `cargo test`

## 里程碑（Milestones）

- [x] M1: 明确外部接口契约与口径（含样例数据/时区/回退策略）。
- [x] M2: 数据库结构与增量计算方案确定（来源区分 + 日统计快照）。
- [x] M3: 统计 API/SSE 合并口径方案与测试清单确认。
