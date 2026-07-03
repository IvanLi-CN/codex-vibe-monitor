# SSE 驱动的请求记录与统计实时更新 - Implementation

## Current State

- Canonical spec: `docs/specs/5932d-sse-proxy-live-sync/SPEC.md`
- Implementation summary: 已完成
- Dashboard realtime consumption now separates SSE-fast KPI commits from 5s HTTP/chart reconcile budgets. Working conversations batch visible SSE patches for 1s and throttle head/snapshot reconcile to 5s. `/api/stats/parallel-work` keeps its response schema while supporting ETag / 304 conditional fetches.
- Current-window summary reconcile now matches the same 5s budget as calendar windows and account-activity reconcile. The dashboard debug diagnostics also split out `current summary refresh/open-resync` and `upstream account activity refresh/open-resync`, so future regressions can distinguish SSE-fast-path churn from HTTP reconcile churn.
- Dashboard working conversations live head/count now read from a write-side `prompt_cache_working_set_live` table that keeps the last 5 minutes of terminal activity plus any current in-flight keys. The public response shape stays unchanged, while the hot read path no longer rebuilds the working set from `codex_invocations` on every request.
- Working-conversations snapshot count/page now also accept the same `<=5s` bounded-freshness contract. Instead of strict historical recomputation from `codex_invocations`, snapshot aggregates read the live working-set truth directly and keep the existing response fields, cursor shape, and main ordering semantics.
- Proxy capture request completion now keeps terminal `codex_invocations` as the synchronous source of truth, but moves bounded derived writes through a process-local SQLite batch writer. Attempt phase/latency progress, invocation hourly rollup/live progress, upstream account activity touch, and background system-task finish updates coalesce on short windows before touching SQLite. Terminal attempt finalize remains synchronous and overwrites any unflushed progress.
- Proxy capture terminal persistence now prefers a narrow update of an existing `running/pending` invocation row and only falls back to guarded `INSERT OR IGNORE` for missing rows. Snapshot/broadcast follow-up treats SQLite locked errors as fail-soft skips with structured evidence, relying on SSE and the normal reconcile loop to catch up instead of blocking proxy completion.
- Runtime proxy snapshots no longer write `codex_invocations` on the normal `running` path. They update a process-local runtime invocation store keyed by `invokeId + occurredAt`, broadcast the in-memory `records` payload immediately, and let terminal proxy capture synchronously overwrite the final main fact before removing the memory row. HTTP current-window records, summary, timeseries, and account-activity in-flight reads overlay the same memory store on top of DB results so open-resync does not briefly drop visible running rows. P2 running snapshots are skipped during shutdown drain instead of being forced back into SQLite.
- Pool account `last_selected_at` selection touches now use an in-process routing fairness anchor plus a coalesced batch write. Candidate sorting overlays the runtime timestamp on top of persisted account rows, while status/cooldown/failure writes remain synchronous because they affect routing correctness.

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-02-25
- Last: 2026-02-25

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- 运行并通过与改动直接相关的 Rust 测试（覆盖代理落库后广播路径）。
- 运行并通过前端构建或测试校验（至少一种自动化验证）。

## Migrated Implementation Sections

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 抽取落库后广播 helper，并改造 `persist_proxy_capture_record` 返回语义支持“是否新插入”。
- [x] M2: 替换代理链路 5 处落库调用点为统一 helper。
- [x] M3: 前端 `useInvocationStream` 增加 SSE open 后静默回源补齐。
- [x] M4: 完成验证、提交、PR、checks 与 review-loop 收敛（fast-track）。
- [x] M5: Dashboard realtime consumers split visible patch, KPI, chart commit, head reconcile, and parallel-work conditional-fetch budgets.
- [x] M6: 活动调用记录列表统一接入 `records` SSE：`Live`、`/records` 与账号详情抽屉 records tab 现在共用一套记录过滤、去重、终态优选与 SSE open 静默回源逻辑。
- [x] M7: Proxy capture 派生写与 attempt 中间进度进入 SQLite batch writer；保持代理并发与 terminal 主事实同步落盘不变。
- [x] M8: Proxy capture terminal 主事实写入改为 existing-row 窄更新优先，DB locked snapshot/broadcast 改为 fail-soft skip；公开 SSE/API shape 不变。
- [x] M9: Runtime running snapshot 去同步主表写；账号选择 touch 去耦到内存公平性锚点 + batch writer，terminal 主事实与路由状态正确性保持同步可靠。
- [x] M10: Runtime running snapshot 收口为纯内存实时态 + SSE/HTTP overlay；DB 只保留首次极窄恢复占位，后续 running refresh 不再常规写主表，shutdown 只 drain P0/P1，P2 running snapshot 仅记录 skip 证据。

## 2026-06-21 Follow-up

- 新增 `web/src/lib/invocationRecordsLive.ts`，把活动记录窗口的过滤、排序、去重与“更完整终态记录优选”抽成共享工具，避免 `Live`、账号详情抽屉和 `/records` 页各维护一套实时合并语义。
- 新增 `web/src/hooks/useInvocationRecordsRealtime.ts`，统一负责 `records` SSE 订阅、已命中窗口内的可见记录合并，以及 SSE `open` 后静默 reconcile。
- 账号详情抽屉 records tab 不再只做一次性 `fetchInvocationRecords(...)`；它现在按 `upstreamAccountId + limit + tab/open lifecycle` 受控订阅 SSE，并在连接恢复后静默回源补齐。
- `/records` 页保留原有筛选、分页、排序、`snapshotId` 与 `newRecordsCount` 语义，同时只把“命中当前窗口”的 SSE 记录合并进当前页；窗口外增量继续通过 `New data` 提示暴露，不静默污染当前结果集。

## Verification

- `cd web && bun run test -- --run src/hooks/useInvocations.test.tsx src/hooks/useInvocationRecords.test.tsx src/pages/account-pool/UpstreamAccounts.test.tsx`
- `cd web && bun run test useStats.test.ts`
- `cd web && bun run test useDashboardUpstreamAccountActivity.test.tsx`
- `cargo test sqlite_batch_writer`
- `cargo test persist_and_broadcast_proxy_capture_runtime_snapshot_emits_queryable_running_record -- --nocapture`
- `cargo test runtime_snapshot_batches_prompt_cache_rollups_without_background_follow_up -- --nocapture`
- `cargo test persist_and_broadcast_proxy_capture_runtime_snapshot_uses_memory_overlay_without_sync_db_write --no-fail-fast`
- `cargo test shutdown_drain_skips_running_proxy_snapshots --no-fail-fast`
- `cargo test timeseries --no-fail-fast`
- `cargo test resolver_ -- --nocapture`
- `cargo test pool_upstream_request_attempt -- --test-threads=1`
