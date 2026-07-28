---
title: Durable multi-granularity long-term usage rollups
module: stats
problem_type: correctness
component: long-term usage analytics
tags:
  - rollup
  - archive
  - sqlite
  - wall-time
  - retention
status: active
related_specs:
  - docs/specs/5k89c-long-term-usage-analytics/SPEC.md
  - docs/specs/9aucy-db-retention-archive/SPEC.md
  - docs/specs/z9h7v-invocation-log-observability/SPEC.md
---

# Durable multi-granularity long-term usage rollups

## Context

Long-lived usage views must remain complete after invocation rows move from the live SQLite table into compressed archive batches. A single online query over live and archive data makes the response latency and retention correctness depend on disk scans and decompression.

## Resolution

- Materialize the same invocation into isolated `overall`, `model` and `upstream` hourly and daily dimensions.
- Treat daily rows as the permanent read model. Rebuild them idempotently from live rows plus every readable completed archive; retain hourly rows only inside the configured window, clamped to at least 366 days.
- Persist a state row with `preparing/running/ready/error`, processed and total row counts, and the earliest reconstructable Shanghai date. While state is not ready, APIs return preparation metadata without partial totals.
- Mark each archive batch with a dedicated replay target only after the new read model transaction commits. Archive cleanup must require that marker in addition to existing historical-rollup gates.
- Compute wall time as interval unions per dimension and hour. Slice intervals at hour boundaries before persisting so concurrent calls and cross-account overlap are not double-counted when daily rows are read back.
- Keep missing or unreadable archive batches out of the result by advancing the persisted start date past their coverage end; leave their cleanup marker absent so a later retry can recover them.
- Treat `invocation_rollup_hourly` as an integrity oracle for the `overall` count, Token and cost totals, never as the source for model or upstream dimensions. Audit retained completed Shanghai days hourly, including nonzero materialized days for which the oracle has no rows, queue mismatches durably, and replace a date only after a live/archive reconstruction matches both its daily and per-hour oracle totals.

## Guardrails / Reuse Notes

- Never use an aggregate row's `wall_time_ms` as a request count; preserve separate samples for every metric.
- Token and cost totals are sums of non-null samples, while performance metrics only accept successful rows with valid timings. Output speed is weighted by output tokens divided by stream duration, not an average of per-call speeds.
- Archive reads should enrich historical API Key identity from the soft-deleted account row. Missing or non-API-Key identities belong to the stable `other` series.
- API series keys must be issued by the bounded overview and validated against the same date window; reject more than eight keys before querying daily points.
- Removing a date from the full-rebuild replacement set is insufficient protection: remove that date from every partial and rebuilt candidate map before any UPSERT. Otherwise a partial candidate can still overwrite a retained complete row.
- A failed integrity repair must preserve existing rollups, or preserve an empty result when no durable row exists, keep the existing error state visible until its queue entry is cleared, and retry with persisted bounded backoff. SQLite `BUSY/LOCKED` retries stay inside the low-priority long-term refresh path and must not redesign global writer behavior.

## References

- `src/long_term_stats.rs`
- `src/maintenance/archive/cleanup.rs`
- `docs/specs/5k89c-long-term-usage-analytics/SPEC.md`
