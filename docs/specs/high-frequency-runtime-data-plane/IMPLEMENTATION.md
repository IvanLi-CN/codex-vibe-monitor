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

`SerializedTopicFrame` is now materialized directly from typed Dashboard bases and immutable projection slices:

- `DashboardTopicMaterializer` retains one revision-aware typed base per cached topic and derives a `DashboardTopicRevision` from the base cursor plus only its dependencies: activity uses current, network and terminal; summary uses current and terminal; network timeseries/recent use network. Activity and summary mutate their typed bases in place, so their Auto revisions do not deep-clone the cached response.
- Activity terminal bases retain the aggregate state not present in the response wire shape. Each terminal slice updates total stats and accumulators first, then derives model performance and account latency once per affected aggregate while preserving the bounded recent-invocation projection; persisted baselines carry their queued terminal sequence so the same shared slice is not replayed. This requires neither SQLite reads nor complete invocation broadcasts.
- In `auto` mode, the producer broadcasts `DashboardCurrentSlice`, `DashboardNetworkSlice` or `DashboardTerminalSlice`. The subscription hub serializes each affected topic revision once, commits one shared `Arc<SerializedTopicFrame>` to cache/replay/broadcast, and SSE owners retain frame references rather than business payloads or mutable generic JSON.
- Incoming slices and detached materialization commits are monotonic: stale slice revisions are rejected and the dependency graph is revalidated under the hub lock before a frame can update cache, replay or cursor state.
- Revision delivery never rebuilds a topic base, reads SQLite, or reconciles. Network timeseries serializes its typed base by borrowing every retained point and substituting only the current slice's live point; network recent serializes the current slice by reference. In `auto`, the shared network projection producer owns the fixed `1s` cadence for both network topics, so subscription tasks do not run a second producer. Byte-identical output retains its frame and cursor. Subscriber-free topics remain dirty and rebuild an authoritative base when ownership returns.
- Revision delivery never rebuilds a topic base, reads SQLite, or reconciles. Network timeseries serializes its typed base by borrowing every retained point and substituting only the current slice's live point; network recent serializes the current slice by reference. Terminal totals apply their typed delta to activity and open summary bases on the fixed `5s` slice. In `auto`, the shared network projection producer owns the fixed `1s` cadence for both network topics, so subscription tasks do not run a second producer. Byte-identical output retains its frame and cursor. Subscriber-free topics remain dirty and rebuild an authoritative base when ownership returns.
- The `legacy` kill switch keeps the pre-existing `DashboardActivityLive`, JSON-overlay, network-recent subscription cadence, and Records-refresh terminal delivery paths intact for one release.
- The full topology contract opens two real `topic_sse_stream` Dashboard connections, verifies one shared frame identity for activity, summary, network timeseries and network recent, asserts zero business-payload broadcasts, JSON overlays, and complete payload clones, one serialization per materialized revision, zero live-path SQLite reads, and no lag or skipped frames. Focused coverage exercises terminal revision idempotence, cold network-only materialization, repeated-network-revision suppression, auto/legacy cadence routing, a legacy recent cadence frame through the SSE entrypoint, revision independence and the current-state p95 gate.

Runtime projection maintains independent current/phase, network/rate and terminal-total dirty generations, revisions and non-extending `250ms`, `1s` and `5s` deadlines. Network-only changes do not build or advance the current slice; active network topics rearm only the network cadence so rates and recent windows decay without waking current projection. Terminal slice staging is bounded and drained on its fixed deadline even without subscribers, preventing subscriber-free retention.

Aggregate validation remains responsible for full backend/web/Storybook coverage, controlled performance evidence, review convergence and owner-approved browser viewport evidence.

Runtime pressure diagnostics are implemented for issue #738:

- `GET /api/system/status` exposes additive `runtimePressureHealth` assembled from existing in-memory projection, request-pipeline, process-memory and writer-accounting counters without adding status-page SQL.
- The System Status workspace treats a missing field as unknown and presents healthy, deferred, degraded and accounting-error summaries with expandable, non-sensitive details.
- Storybook and the mock-only Web Demo provide deterministic states for contract, responsive and visual regression coverage.
