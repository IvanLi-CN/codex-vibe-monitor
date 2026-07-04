# SSE 驱动的请求记录与统计实时更新 - Implementation

## Current State

- Canonical spec: `docs/specs/5932d-sse-proxy-live-sync/SPEC.md`
- Implementation summary: 已完成
- Dashboard realtime consumption now separates SSE-fast KPI commits from 5s HTTP/chart reconcile budgets. Working conversations batch visible SSE patches for 1s and throttle head/snapshot reconcile to 5s. `/api/stats/parallel-work` keeps its response schema while supporting ETag / 304 conditional fetches.
- Current-window summary reconcile now matches the same 5s budget as calendar windows and account-activity reconcile. The dashboard debug diagnostics also split out `current summary refresh/open-resync` and `upstream account activity refresh/open-resync`, so future regressions can distinguish SSE-fast-path churn from HTTP reconcile churn.
- Dashboard working conversations live head/count now read from a write-side `prompt_cache_working_set_live` table that keeps the last 5 minutes of terminal activity plus any current in-flight keys. The public response shape stays unchanged, while the hot read path no longer rebuilds the working set from `codex_invocations` on every request.
- Working-conversations snapshot count/page now also accept the same `<=5s` bounded-freshness contract. Instead of strict historical recomputation from `codex_invocations`, snapshot aggregates read the live working-set truth directly and keep the existing response fields, cursor shape, and main ordering semantics.
- Proxy capture request completion no longer waits for terminal `codex_invocations` SQLite persistence before allowing the proxy business flow to finish. It constructs the full terminal record, tombstones/removes the corresponding runtime-store row, broadcasts `records`, and enqueues the terminal record into the SQLite write controller as P1 best-effort observability. Enqueue/flush failures are structured evidence and must not fail an already completed proxy response.
- The SQLite write controller is the single write path for terminal invocation records and bounded derived writes. Terminal records flush first; terminal-generated rollup/account-touch maintenance is retained as deferred P2 work for a later controller window instead of running in the same lock window. Attempt phase/latency progress, invocation hourly rollup/live progress, upstream account activity touch, and background system-task finish updates continue to coalesce on short windows before touching SQLite. Terminal attempt begin/finalize remains synchronous for now because the existing failover/recovery flow depends on a concrete attempt id and final attempt state.
- Runtime proxy snapshots no longer write `codex_invocations` or enqueue recovery placeholders on the normal `running` path. They update a process-local runtime invocation store keyed by `invokeId + occurredAt` and broadcast the in-memory `records` payload immediately. HTTP current-window records, summary, timeseries, account-activity in-flight reads, and prompt-cache working conversations overlay the same memory store on top of DB results so open-resync does not briefly drop visible running rows. P2 running snapshots are skipped during shutdown drain instead of being forced back into SQLite.
- Tracked proxy capture requests now create an admit-time shell `running` record immediately after `invokeId + occurredAt` assignment and header inspection, before request body read, proxy settings read, account routing, or upstream attempt start. Later body-parsed and attempt snapshots upsert the same runtime-store key to enrich model, prompt-cache, account, and timing fields instead of adding another visible row.
- The `/v1/*` request entry path now emits that shell before route-context resolution as well. Route validation failures terminalize the same runtime-store key with a terminal overlay, so invalid pool/key failures do not leave a false `running` row while SQLite record persistence remains best-effort.
- Terminal record follow-up no longer forces an immediate SQLite batch flush barrier. With active subscribers, the terminal overlay is broadcast first and summary/quota follow-up is deferred briefly without blocking business response or forcing write-controller lock acquisition.
- If a request future drops after the admit-time runtime row but before any terminal invocation exists, the drop guard terminalizes the same runtime-store key with an interrupted overlay and broadcasts `records`; it must not silently remove the row because clients preserve transient `id=0` in-flight records across HTTP reconcile.
- Runtime overlay is intentionally unbounded by activity windows for `running/pending` rows. Current summary in-progress counts, account-activity in-flight cards, and working-conversation current cards all include memory rows regardless of start time; only terminal/historical DB rows remain constrained by the selected window.
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
- [x] M7: Proxy capture 派生写与 attempt 中间进度进入 SQLite batch writer；保持代理并发不变。
- [x] M8: Proxy capture terminal 主事实写入改为 existing-row 窄更新优先，DB locked snapshot/broadcast 改为 fail-soft skip；公开 SSE/API shape 不变。
- [x] M9: Runtime running snapshot 去同步主表写；账号选择 touch 去耦到内存公平性锚点 + batch writer，路由状态正确性保持同步可靠。
- [x] M10: Runtime running snapshot 收口为纯内存实时态 + SSE/HTTP overlay；后续 running refresh 不再常规写主表，shutdown 不强制 flush P2 running snapshot。
- [x] M11: Terminal invocation 记录从代理业务关键路径移入 SQLite write controller；业务响应不等待记录落库，running snapshot 完全退出 DB/batch 路径，terminal 产生的派生写延后到后续 P2 flush。
- [x] M12: Proxy capture 请求 admit 后立即创建最小内存 running shell record；body parse / attempt start / response-ready 只覆盖补全同一 runtime key。

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
- `cargo test admitted_proxy_capture_snapshot_is_visible_before_body_parse_and_later_enriched -- --nocapture`
- `cargo test runtime_snapshot_batches_prompt_cache_rollups_without_background_follow_up -- --nocapture`
- `cargo test persist_and_broadcast_proxy_capture_runtime_snapshot_uses_memory_overlay_without_sync_db_write --no-fail-fast`
- `cargo test persist_and_broadcast_proxy_capture_runtime_snapshot_uses_memory_overlay_without_sync_db_write --quiet -- --test-threads=1`
- `cargo test shutdown_drain_skips_running_proxy_snapshots --no-fail-fast`
- `cargo test timeseries --no-fail-fast`
- `cargo test resolver_ -- --nocapture`
- `cargo test pool_upstream_request_attempt -- --test-threads=1`
- `cargo test proxy_openai_v1_invalid_pool_key_bypasses_admission_backpressure -- --nocapture`
