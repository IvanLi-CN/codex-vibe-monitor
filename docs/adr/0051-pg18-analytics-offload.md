# Optional PostgreSQL Analytics Offload

Status: Accepted

## Context

Long-term analytics already has a SQLite projection and a compatibility contract for daily and hourly metrics. A future deployment may need a separately provisioned PostgreSQL read/offload store, but introducing that store must not change the existing SQLite state machine, business writes, retention guarantees, or raw-data safety. The foundation also needs to distinguish “PostgreSQL was selected but is unavailable” from “SQLite is the configured backend”.

This decision is intentionally a boundary decision. It does not claim a throughput improvement or establish a production deployment plan; validation is limited to deterministic configuration, schema, serialization, and short runtime-contract checks.

## Decision

- Select the analytics backend explicitly with `ANALYTICS_BACKEND=sqlite|postgres`, defaulting to `sqlite`.
- Keep the existing SQLite database URL and long-term projection as the no-configuration behavior. Do not add an implicit heavy SQLite fallback after PostgreSQL has been explicitly selected and found unavailable.
- Accept `POSTGRES_DATABASE_URL` only through an opaque, redacting wrapper. Use `POSTGRES_MAX_CONNECTIONS=5` by default and reject zero or unsupported values.
- Keep the PostgreSQL schema in the separate `cvm_analytics` namespace. Its normalized facts include only the identity, dimension, terminal, token, cost, timing, and state fields required by the long-term analytics contract. Raw payloads, prompts, requester IPs, and unbounded error text remain outside this store; persisted state uses an error code rather than an error body.
- Provisioning is explicit. `PG_ANALYTICS_SCHEMA_SQL` is a reviewable schema contract; application startup never runs its DDL or business writes. A selected PostgreSQL runtime verifies the required relations and executes a disposable read-only transaction before becoming ready.
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
