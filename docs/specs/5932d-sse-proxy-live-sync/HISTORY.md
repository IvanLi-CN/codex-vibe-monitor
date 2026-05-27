# SSE 驱动的请求记录与统计实时更新 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/5932d-sse-proxy-live-sync/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.
- Dashboard live updates separate fast visible paths from heavier reconcile paths: SSE summary updates KPI numbers immediately, working conversations apply 1s visible patch batches, and chart/head/aggregate reconciles use a 5s budget.
- `parallel-work` keeps its existing response JSON shape; bandwidth reduction is handled through ETag / 304 conditional HTTP rather than trimming fields.
