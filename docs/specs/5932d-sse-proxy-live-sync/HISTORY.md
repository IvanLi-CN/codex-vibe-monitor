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
- 2026-06-30: 第二轮 CPU 止血把 Dashboard working conversations 当前 head/count 的真相源前移到 write-side `prompt_cache_working_set_live`，接受 `<=5s` bounded freshness，但不再让 5 分钟工作集每次 reconcile 都重扫 `codex_invocations`。
- 2026-06-30: 第三轮 CPU 止血继续把 working conversations 的 snapshot count/page 从 `WITH recent_terminal` 历史重算收口到 live working-set truth。接口继续保留 `snapshot_at`、cursor 与字段 shape，但 snapshot 聚合不再承诺严格历史时点重算，只承诺 `<=5s` bounded freshness。
- 2026-07-01: 第四轮 SQLite 止血明确不降低代理并发，也不把 terminal `codex_invocations` 主事实改成 write-behind；仅把 attempt 中间进度、invocation rollup/live progress、upstream account touch 与 system task finish 这类可接受 `<=5s` 新鲜度的派生写放入有界 batch writer。
