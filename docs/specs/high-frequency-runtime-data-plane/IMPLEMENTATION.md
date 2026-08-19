# High-Frequency Runtime Data Plane Implementation

## Dashboard Hot Topic Coverage

Activity、summary 与 network topic 已有 typed projection/materializer。working-conversations、parallel-work open range 与 open-window timeseries 仍可能进入通用 subscription builder；其迁移、健康判责与完整 Dashboard bundle 性能门禁由 [`dashboard-hot-topic-projection`](../dashboard-hot-topic-projection/IMPLEMENTATION.md) 跟踪。在该规范验收前，高频 Dashboard delivery 不视为全部完成。

`GET /api/stats/summary` 使用启动 hydration 的 `SummaryProjection` 作为唯一 HTTP 真值。projection 保留按账号和 UTC bucket 的 rollup/archive coverage、usage/model 与延迟聚合、bounded recent-N 索引、48 小时 live exact tail、受支持 rolling/calendar 窗口的归档边界行、runtime overlay 和 maintenance last-good；完整历史由 compact hourly rollup 与 account coverage 派生，归档在后台流式归并后释放。公开 rolling duration 限制为 30 天；48 小时以内允许任意分钟/小时粒度，超过 48 小时仅接受整日粒度，其他输入在 handler 参数验证阶段返回既有 400，不进入 hub、projection 或 SQLite。请求仅规范化选择并在内存中派生精确 `StatsResponse`。48 条 LRU 只缓存已序列化响应，不能限制受支持选择。typed runtime/persistence events 进入合并触发器；后台维护器以单飞、250ms debounce、10 秒最小间隔和 10 秒 deadline 发布新 revision，并在 15 秒 freshness 预算内保留 exact last-good；请求不执行 SQLite 或文件 I/O，刷新失败超过预算采用端点 unavailable 契约。

## Typed Runtime Event Bus Boundary

The next delivery boundary is a single typed runtime mutation bus and router. Hot events carry identity, lifecycle, aggregate, and cursor fields only; they do not carry full invocation records, generic JSON values, or mutable topic snapshots. Topic work is selected from active dependency indexes before any materialization. Historical and detail consumers use bounded identity hydration.

Backfill scheduling is event-driven. `startup_backfill_progress` persists each task cursor, `next_run_after`, source-unavailable state, daily probe deadline and wake generation; the supervisor mirrors the recovered per-task deadlines in memory, then sleeps until the earliest deadline or a matching task wake. Archive materialization wakes only archive-activity and historical-rollup work. A source-unavailable probe remains bounded to 100 rows or two seconds, while pressure deferral persists a retry deadline without producing a `system_task_runs` audit row. A no-op pass does not run unrelated rollup maintenance or create a task-run audit row. Health aggregates event-bus lag, projection recovery, and writer pressure using in-memory counters without adding status-page SQL.

`ProxySqliteWriteCoordinator` 是代理热写的统一 admission 面。实现覆盖 terminal P1/P2 actor、attempt 生命周期和 route success/failure 汇聚路径，并通过 `runtimePressureHealth.proxySqliteWriteCoordinator` 暴露 active class、各优先级 waiter 与 legacy bypass 计数。

P1 正常 admission 为 20ms，批次上限为 32 条或 4 MiB；失败后按 250ms 到 5s 退避。P2 使用独立 250ms 固定 deadline，pressure defer、background busy 和实际 lock retry 分开调度与计数；hourly replay 每次只执行一个既有有界 chunk，未覆盖 derived work 保留到下一轮。

Prompt Cache window topic 在首个 owner 订阅时建立精确 baseline。后续 typed runtime mutations 按调用 identity 去重并在 500ms 后直接更新 cached payload；完整 hydrate 仅用于初始 baseline、60 秒 reconcile 或 dirty recovery。最后一个 owner 释放时丢弃 pending delta 并要求重订阅 fresh baseline。

## Delivery Topology

- Integration branch: `prd/typed-runtime-event-bus`
- Final base: `main`
- Child merge policy: risk-gated
- Final merge policy: owner-explicit
- Child work is tracked by GitHub Issues and targets the integration branch.

`BroadcastPayload::Records` and the related prompt-cache and attempt variants are absent from the production binary. `#[cfg(test)]` observer shims preserve legacy persistence assertions while tests migrate; they never subscribe to, publish to, or affect the typed runtime mutation router.

## Module Boundaries

- Request ingress and semantic projection: `src/proxy/dispatch.rs`, `src/proxy/stream_gate.rs` and a dedicated semantic projection module.
- Runtime projection: a dedicated `RuntimeProjectionHub` owned by application state; Dashboard renderers depend on this Hub rather than SQLite.
- Terminal durability: existing journal, SQLite batch writer and `TerminalProjectionHub`; queue accounting moves behind `PendingQueueAccounting`.
- Delivery: subscription cache, replay and fan-out share immutable serialized frames.
- Health: in-memory counters feed `runtimePressureHealth`; System Status only formats those counters.

## Migration Sequence

1. Make writer accounting and process diagnostics trustworthy before using them as gates.
2. Replace duplicate request parsing/materialization with one semantic projection while preserving its explicit compatibility behavior.
3. Move Dashboard live rendering to Runtime Projection and prove zero live-path DB reads.
4. Replace mutable JSON topic fan-out with shared serialized frames and subscriber reference gating.
5. Expose health states in System Status, complete visual evidence, then remove obsolete production paths after A/B evidence.

## Compatibility

- The typed runtime mutation bus is mandatory in production. Legacy Dashboard and Prompt Cache runtime-bus environment modes are rejected and cannot re-enable complete-record broadcasts.
- HTTP/SSE payload contracts are unchanged; additive System Status data is optional to clients.
- Existing persistence, terminal journal and closed-range builders remain authoritative recovery paths.

## `/v1/responses` Live Request Body

- Runtime setting keeps live request-body streaming disabled by default. When enabled, eligible `/v1/responses` requests are split deterministically into buffered control and v2 treatment cohorts without an account-group filter; v1 data is excluded from the v2 comparison.
- The request pipeline reuses the raw replay snapshot for retries, validates the complete root JSON object before publishing the final route, then applies JSON/compression transforms. No upstream body exists while a route can still change; bounded parsing, cold cache and EOF finalization preserve the buffered contract.
- The runtime snapshot carries the live setting, account rows, candidates, rules, capability state, bindings, sticky routes, route penalties and model health needed for selection. Prompt-cache/encrypted-owner and sticky-route lookups use independent 16,384-entry negative LRUs with single-flight cold reads and write-through invalidation. Failures and recoveries publish a refreshed routing snapshot; healthy success acknowledgements do not trigger a reload. The performance response reports route-finalization byte positions, ratios, factors and cache outcomes separately from successful response-benefit samples.
- Invocation persistence and hourly statistics retain exact request/response overlap timestamps so the performance surface compares direct first-response and first-token measurements instead of additive stage estimates.

## Verification State

Runtime Projection is implemented through `RuntimeProjectionHub` and `DashboardLiveProjection`:

- Runtime, phase, account metadata, network and terminal mutations feed one in-memory current-state projection.
- Healthy Dashboard current-state rendering has no SQLite dependency; persistence is isolated to startup restore, the pressure-gated 60-second reconcile and explicit cold fallback.
- Producer updates use a non-extending 250-millisecond deadline, retain last-good data on degraded paths and suppress unchanged revisions.
- Runtime pressure health exposes projection mode/state, producer/subscriber state, live-path database reads, build count, revision, snapshot origin and last-good age without querying SQLite.
- Tests cover 10,000 healthy mutations with zero live-path database reads, a current-state update p95 at or below 400 milliseconds, cold fallback and degraded last-good behavior.
- Runtime mutations update a compact account-level live aggregate in place. The 250-millisecond producer clones only the bounded account snapshot and overlays network counters; it does not clone or traverse retained `ApiInvocation` records.
- `runtimePressureHealth.requestPipeline` exposes the active mode, latest snapshot kind, cumulative semantic parse and whole-body materialization counts, rewrite buffer peak and latest fallback reason from in-memory counters.

`SerializedTopicFrame` is now materialized directly from typed Dashboard bases and immutable projection slices:

- `DashboardTopicMaterializer` retains one revision-aware typed base per cached topic and derives a `DashboardTopicRevision` from the base cursor plus only its dependencies: activity uses current, network and terminal; summary uses current and terminal; network timeseries/recent use network. Activity and summary mutate their typed bases in place, so their Auto revisions do not deep-clone the cached response.
- Activity terminal bases retain the aggregate state not present in the response wire shape. Each terminal slice indexes accounts once, updates total stats and accumulators first, then derives model performance and account latency once per affected aggregate while preserving the bounded recent-invocation projection; persisted baselines carry their queued terminal sequence so the same shared slice is not replayed. Open summary bases capture their SQL response and terminal watermark behind the terminal-writer barrier, then overlay only unacknowledged deltas. This requires neither SQLite reads nor complete invocation broadcasts during revision delivery.
- In `auto` mode, the producer broadcasts `DashboardCurrentSlice`, `DashboardNetworkSlice` or `DashboardTerminalSlice`. The subscription hub serializes each affected topic revision once, commits one shared `Arc<SerializedTopicFrame>` to cache/replay/broadcast, and SSE owners retain frame references rather than business payloads or mutable generic JSON.
- Incoming slices and detached materialization commits are monotonic: stale slice revisions are rejected and the dependency graph is revalidated under the hub lock before a frame can update cache, replay or cursor state.
- Revision delivery never rebuilds a topic base, reads SQLite, or reconciles. Network timeseries serializes its typed base by borrowing every retained point and substituting only the current slice's live point; network recent serializes the current slice by reference. Terminal totals apply their typed delta to activity and open summary bases on the fixed `5s` slice. The producer-owned `60s` runtime reconcile scans activity calendar transitions plus activity/summary rolling-duration bases, then rebuilds active stale bases outside the subscription task; activity retains a rebase-only range anchor so a terminal slice can advance its public range without deferring that rebuild. Terminal delivery only marks a stale base dirty and skips its slice until that replacement succeeds. A fresh rolling base never schedules SQLite work, a failed rebase remains isolated for the next producer reconcile, and a byte-identical rebase retains its frame and cursor. In `auto`, the shared network projection producer owns the fixed `1s` cadence for both network topics, so subscription tasks do not run a second producer. Subscriber-free topics remain dirty and rebuild an authoritative base when ownership returns.
- The runtime router receives compact mutation events only. It selects active topic dependencies before materialization, sends Dashboard slices to their dedicated projection materializers, and uses bounded durable identity hydration only for cold detail consumers.
- The full topology contract opens two real `topic_sse_stream` Dashboard connections, verifies one shared frame identity for activity, summary, network timeseries and network recent, asserts zero business-payload broadcasts, JSON overlays, and complete payload clones, one serialization per materialized revision, zero live-path SQLite reads, and no lag or skipped frames. Focused coverage exercises terminal revision idempotence, cold network-only materialization, repeated-network-revision suppression, typed-router active-topic filtering, cursor-gap recovery, revision independence and the current-state p95 gate.

Runtime projection maintains independent current/phase, network/rate and terminal-total dirty generations, revisions and non-extending `250ms`, `1s` and `5s` deadlines. Network-only changes do not build or advance the current slice; active network topics rearm only the network cadence so rates and recent windows decay without waking current projection. Terminal slice staging is bounded and drained on its fixed deadline even without subscribers, preventing subscriber-free retention.

Aggregate validation remains responsible for full backend/web/Storybook coverage, controlled performance evidence, review convergence and owner-approved browser viewport evidence.

Runtime pressure diagnostics are implemented for issues #738 and #768:

- `GET /api/system/status` exposes additive `runtimePressureHealth` assembled from existing in-memory projection, request-pipeline, process-memory and writer-accounting counters without adding status-page SQL.
- `runtimePressureHealth.eventBus` reports typed-router publication, coalescing, active topic work, lag/gap recovery and payload-clone counters. `runtimePressureHealth.backfill` reports recovered startup-backfill wakes, due dispatches, suppressed no-op passes, pressure deferrals and active deferred or failed tasks. Event lag, projection deferral, cursor growth and writer pressure keep the aggregate state out of `healthy`.
- The System Status workspace treats a missing field as unknown and presents healthy, deferred, degraded and accounting-error summaries with expandable, non-sensitive details.
- Storybook and the mock-only Web Demo provide deterministic states for contract, responsive and visual regression coverage.
