# SSE 驱动的请求记录与统计实时更新 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/5932d-sse-proxy-live-sync/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.
- Dashboard live updates separate fast visible paths from heavier reconcile paths: SSE summary updates KPI numbers immediately, working conversations apply 1s visible patch batches, and chart/head/aggregate reconciles use a 5s budget.
- `parallel-work` keeps its existing response JSON shape; bandwidth reduction is handled through ETag / 304 conditional HTTP rather than trimming fields.
- 2026-06-21: 继续把“活动中的调用记录列表”统一收口到现有 `records` SSE + open 后静默回源链路，明确覆盖 `Live`、`/records` 与账号详情抽屉 records tab；历史回放类抽屉保留各自 snapshot/history 语义，不强行改造成全量实时流。
- 2026-06-29: Dashboard current-window summary reconcile 不再保留比 calendar window 更激进的 cadence；`current summary` 与 `upstream account activity` 统一收口到 `5s` refresh/open-resync 预算，避免前端更快回源把后端请求级 SQLite 热点放大成持续 CPU 压力。
