# Dashboard SSE 更新链路优化 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/xvdhm-dashboard-sse-refresh-optimization/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-07: 创建规格并冻结优化边界、验收标准与快车道交付目标。
- 2026-03-07: 完成后端 changed-only 广播与前端 summary/timeseries 刷新策略收敛，`cargo test`、`cd web && npm test`、`cd web && npm run build` 通过。
- 2026-03-07: 浏览器实测 `/dashboard`，确认 records 推送后 recent table / `today` / `24h` / `7d` / `90d` 当前桶可见更新，且 reconnect 仅触发一轮静默 backfill 请求。
- 2026-03-07: 创建 PR #90，补齐 `type:patch` + `channel:stable` labels，并确认 Label Gate / CI Pipeline 全绿；review 复查未发现阻塞项。
