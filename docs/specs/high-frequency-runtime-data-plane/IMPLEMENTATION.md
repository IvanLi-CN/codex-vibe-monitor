# High-Frequency Runtime Data Plane Implementation

## Delivery Topology

- Integration branch: `prd/dashboard-runtime-delivery-plane`
- Final base: `main`
- Child merge policy: risk-gated
- Final merge policy: owner-explicit
- Child work is tracked by GitHub Issues and targets the integration branch.

## Module Boundaries

- Request ingress and semantic projection: `src/proxy/dispatch.rs`, `src/proxy/stream_gate.rs` and a dedicated semantic projection module.
- Runtime projection: a dedicated `RuntimeProjectionHub` owned by application state; Dashboard renderers depend on this Hub rather than SQLite.
- Terminal durability: existing journal, SQLite batch writer and `TerminalProjectionHub`; queue accounting moves behind `PendingQueueAccounting`.
- Delivery: subscription cache, replay and fan-out share immutable serialized frames.
- Health: in-memory counters feed `runtimePressureHealth`; System Status only formats those counters.

## Migration Sequence

1. Make writer accounting and process diagnostics trustworthy before using them as gates.
2. Replace duplicate request parsing/materialization with one semantic projection while preserving legacy kill-switch behavior.
3. Move Dashboard live rendering to Runtime Projection and prove zero live-path DB reads.
4. Replace mutable JSON topic fan-out with shared serialized frames and subscriber reference gating.
5. Expose health states in System Status, complete visual evidence, then remove obsolete production paths after A/B evidence.

## Compatibility

- `auto` is the default for both new pipelines. A legacy mode remains available for operational rollback during rollout.
- HTTP/SSE payload contracts are unchanged; additive System Status data is optional to clients.
- Existing persistence, terminal journal and closed-range builders remain authoritative recovery paths.

## Verification State

Runtime Projection is implemented through `RuntimeProjectionHub` and `DashboardLiveProjection`:

- Runtime, phase, account metadata, network and terminal mutations feed one in-memory current-state projection.
- Healthy Dashboard current-state rendering has no SQLite dependency; persistence is isolated to startup restore, the pressure-gated 60-second reconcile and explicit cold fallback.
- Producer updates use a non-extending 250-millisecond deadline, retain last-good data on degraded paths and suppress unchanged revisions.
- Runtime pressure health exposes projection mode/state, producer/subscriber state, live-path database reads, build count, revision, snapshot origin and last-good age without querying SQLite.
- Tests cover 10,000 healthy mutations with zero live-path database reads, a current-state update p95 at or below 400 milliseconds, cold fallback and degraded last-good behavior.
- Runtime mutations update a compact account-level live aggregate in place. The 250-millisecond producer clones only the bounded account snapshot and overlays network counters; it does not clone or traverse retained `ApiInvocation` records.
- `runtimePressureHealth.requestPipeline` exposes the active mode, latest snapshot kind, cumulative semantic parse and whole-body materialization counts, rewrite buffer peak and latest fallback reason from in-memory counters.

`SerializedTopicFrame` 已用于 cache、replay 与 subscriber 共享字节，但 producer 到 topic materialization 的边界尚未完成：

- committed frame 之后的 cache、replay 与 subscriber 已共享不可变 `Arc`。
- Dashboard live producer 仍广播完整业务 snapshot；多个受影响 topic 仍会克隆 cached payload、应用 JSON overlay 并分别序列化。这是当前需要迁移的生产热点，不能标记为已完成。
- Byte-identical projections retain the current frame and cursor. Subscriber-free topics remain dirty and rebuild an authoritative snapshot when ownership returns.
- 现有 focused tests 只覆盖单 topic Arc identity、owner-count scaling、unchanged cursor suppression 和 replay compatibility；后续验证必须覆盖一个 Dashboard tab 同时激活全部相关 topic 的真实 producer 拓扑。

Dashboard full-topology contract opens real `topic_sse_stream` connections for the Dashboard topics and records producer, materialization and serialization amplification. It remains the gate for the later typed-materializer migration; the projection-slice wave does not claim topic-level clone or serialization removal.

Runtime projection now maintains independent current/phase, network/rate and terminal-total dirty generations, revisions and non-extending `250ms`, `1s` and `5s` deadlines. Network-only changes do not build or advance the current slice; active network topics rearm only the network cadence so rates and recent windows decay without waking current projection. Until the topic migration lands, current and network slices are combined into the existing `DashboardActivityLive` compatibility payload so HTTP/SSE behavior remains unchanged; terminal totals continue through the exact Records refresh path. Terminal slice staging is bounded and drained on its fixed deadline even without subscribers, preventing subscriber-free retention. Unit coverage verifies deadline and revision independence, network isolation, terminal hard-limit behavior and unchanged exact refresh semantics.

The next delivery waves will attach the revisioned slices to typed `TopicMaterializer` implementations and generate shared frames directly. The existing `DashboardActivityLive` compatibility handoff remains until that migration is complete and production evidence permits its removal.

Aggregate validation remains responsible for full backend/web/Storybook coverage, controlled performance evidence, review convergence and owner-approved browser viewport evidence.

Runtime pressure diagnostics are implemented for issue #738:

- `GET /api/system/status` exposes additive `runtimePressureHealth` assembled from existing in-memory projection, request-pipeline, process-memory and writer-accounting counters without adding status-page SQL.
- The System Status workspace treats a missing field as unknown and presents healthy, deferred, degraded and accounting-error summaries with expandable, non-sensitive details.
- Storybook and the mock-only Web Demo provide deterministic states for contract, responsive and visual regression coverage.
