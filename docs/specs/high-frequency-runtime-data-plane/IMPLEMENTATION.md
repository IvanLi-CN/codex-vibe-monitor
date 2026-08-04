# High-Frequency Runtime Data Plane Implementation

## Delivery Topology

- Integration branch: `prd/high-frequency-runtime-data-plane`
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

Aggregate validation remains responsible for full backend/web/Storybook coverage, controlled performance evidence, review convergence and owner-approved browser viewport evidence.
