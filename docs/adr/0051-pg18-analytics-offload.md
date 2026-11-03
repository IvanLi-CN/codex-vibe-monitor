# Optional PostgreSQL Analytics Offload

Status: Accepted

## Context

Long-term analytics already has a SQLite projection and a compatibility contract for daily and hourly metrics. A future deployment may need a separately provisioned PostgreSQL read/offload store, but introducing that store must not change the existing SQLite state machine, business writes, retention guarantees, or raw-data safety. The foundation also needs to distinguish “PostgreSQL was selected but is unavailable” from “SQLite is the configured backend”.

This decision is intentionally a boundary decision. It does not claim a throughput improvement or establish a production deployment plan; validation is limited to deterministic configuration, schema, serialization, and short runtime-contract checks.

## Decision

- Select the analytics backend explicitly with `ANALYTICS_BACKEND=sqlite|postgres`, defaulting to `sqlite`.
- Keep the existing SQLite database URL and long-term projection as the no-configuration behavior. Do not add an implicit heavy SQLite fallback after PostgreSQL has been explicitly selected and found unavailable.
- Accept `POSTGRES_DATABASE_URL` only through an opaque, redacting wrapper. Use `POSTGRES_MAX_CONNECTIONS=5` by default and reject zero or unsupported values.
- Keep `POSTGRES_DATABASE_URL` read-oriented and require a separate redacting `POSTGRES_ANALYTICS_MAINTENANCE_URL` for explicit replica control commands. Configuration compares the effective SQLx login role, including URL query overrides, and rejects identical URLs and shared PostgreSQL login roles. Current analytics reads use a separate reader role with exact column-level `SELECT` grants for fact, key, replica-state, and projection-status predicates/results, with no default privileges or rollup access. The maintenance role receives only the reviewed split grants for bounded facts, keys, state, applied changes, and sequence usage, with identity-column reads limited to `MERGE`/conflict match keys and tombstone predicates and no fact-content reads; neither role executes schema DDL.
- Keep the PostgreSQL schema in the separate `cvm_analytics` namespace. Its normalized facts include only the identity, dimension, terminal, token, cost, timing, and state fields required by the long-term analytics contract. Raw payloads, prompts, requester IPs, and unbounded error text remain outside this store; persisted state uses an error code rather than an error body.
- Provisioning is explicit. `PG_ANALYTICS_SCHEMA_SQL` is a reviewable schema contract applied atomically by the schema owner; application startup and replica control commands never run its DDL. A selected PostgreSQL runtime verifies the required relations and executes a disposable repeatable-read, read-only transaction before becoming ready. Replica bootstrap uses the dedicated maintenance URL for bounded target DML only.
- The v2 replica state keeps the shared legacy-visible `service_eligible` flag false as a rollback fence. The owner-applied compatibility contract fences that shared flag and moves a caught-up state to `bootstrapping` before an explicit rebuild, and bootstrap persists the same current-reader fence immediately after acquiring its advisory lock before any preflight can fail. The current program derives effective eligibility from the semantic version, lifecycle state, baseline, and matching checkpoints; both current and older programs therefore fail closed until the bounded rebuild completes, while an older program that ignores the additive version column cannot serve corrected facts under legacy date semantics.
- The SQLite source journal carries a separate `fact_date_semantics_version` marker on each event. Existing rows and older writers default to legacy version `1`; current writers publish version `2` while retaining journal event version `1` and `AnalyticsFactV1` identity. A v2 consumer validates invocation upserts and tombstones before target DML and CAS-invalidates the observed replica when a legacy event arrives, requiring the explicit bounded bootstrap to re-establish compatibility.
- Analytics read admission and all target fact/key queries contributing to one response run in one `REPEATABLE READ, READ ONLY` transaction, so an in-flight read retains one complete pre-fence snapshot instead of observing a partial rebuild or mixing snapshots.
- Represent connection, schema, and transaction failures as `AnalyticsUnavailable`. The runtime retains the unavailable reason in a bounded, redacted form and exposes the state through the future analytics read boundary.
- Add the runtime as an optional `AppState` entry without changing existing SQLite pool ownership, TerminalJournal flow, Summary freshness, retention, or raw capture behavior.

## Consequences

The default path remains compatible with existing databases and deployments. Operators who select PostgreSQL must provision the schema first, and an unavailable PostgreSQL backend makes the PostgreSQL analytics path unavailable instead of silently changing query cost or semantics. The core service can still start with its existing SQLite state because the optional analytics runtime is isolated from the SQLite pool.

The normalized fact contract gives future ingestion and query work a stable boundary, but this foundation does not provide an ingestion adapter, historical backfill, concrete `AnalyticsReadStore`, UI switch, or production migration. Those changes require their own compatibility review against the existing long-term stats and retention specs.

## Compatibility and Re-entry

Deterministic checks must cover no-configuration SQLite behavior, redaction of credentials in debug/serialization/diagnostics, exact public range values, schema relation coverage, and unavailable behavior without fallback. Existing SQLite state construction must continue to compile with the analytics runtime disabled.

If a later change makes PostgreSQL the default, moves business writes, auto-provisions schema, introduces automatic fallback, or changes the long-term metric shape, it must revisit this ADR and the related topic specifications before implementation.

## Related Specifications

- [`docs/specs/pg18-analytics/SPEC.md`](../specs/pg18-analytics/SPEC.md)
- [`docs/specs/5k89c-long-term-usage-analytics/SPEC.md`](../specs/5k89c-long-term-usage-analytics/SPEC.md)
- [`docs/specs/9aucy-db-retention-archive/SPEC.md`](../specs/9aucy-db-retention-archive/SPEC.md)
