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
- Treat missing or unreadable completed archive batches as unavailable source coverage, not as a safe gap. Block candidate UPSERTs from the affected start date onward, preserve materialized rows, and expose the existing error/retry state until the source becomes readable.
- Treat `invocation_rollup_hourly` as an integrity oracle for the `overall` count, Token and cost totals, never as the source for model or upstream dimensions. Keep a terminal-only proof separate from its all-status operational fields so `running` and `pending` calls cannot create permanent false mismatches with long-term usage. A bounded canonical increment must clear that proof; only a reconciliation that reads every completed invocation archive plus live data may restore it. Run that availability reconciliation on every hourly integrity audit even when every bucket is currently trusted. If any archive is missing or unreadable, or the reconciliation lacks a required source table or column, revoke terminal proofs in the reconstructable source window rather than certifying a partial source set and leave the read model in its existing error state; also revoke a previously trusted canonical bucket at or after the integrity-source boundary when it is absent from a complete reconciliation result or its all-status totals disagree with the complete source scan. The one exception is a bucket whose Shanghai date is strictly before the persisted integrity-source boundary: after verified archive cleanup it is intentionally non-reconstructable, so retain its existing terminal proof instead of treating source absence as loss, even while a separate active archive is temporarily unreadable. Queue every affected Shanghai date before publishing another candidate, retain the proof-revocation error until the next full reconciliation, and retry that reconciliation on every subsequent refresh so a restored or corrected source does not wait for the next audit. Legacy rows with unavailable source files remain untrusted rather than becoming zero-value evidence. Before deleting an invocation archive, scan every real source row and require every effective boundary date to parse before calculating the separate integrity-source boundary as the day after the latest wall-time interval end. Before deleting an attempt archive, resolve every account-mapping pair to a readable invocation source with a parseable effective date; retain the attempt archive if that proof is incomplete. Read missing optional legacy `invoke_id` and source-timing columns as bare `NULL` expressions where an outer query adds aliases. Canonical proof reconciliation must also read missing optional invocation fields, including every timing field, as `NULL` and treat an absent `detail_level` as `full`; an old schema must not turn a readable archive into unavailable coverage. Persist the candidate source boundary alongside `cleanup_state=delete_pending` in the manifest while retaining `status=completed` for readable archives. Delete the file first, then in the final immediate transaction advance the global source boundary and delete metadata only after successful removal or confirmed absence; retain the pending cleanup state when either step cannot complete so a later pass can retry. A missing active invocation manifest is durable unavailable-source evidence, not a completed cleanup: retain it until a previously staged `delete_pending` finalization can prove otherwise. When reconciliation cannot read a replayed archive, clear that archive's long-term replay marker so a restored same-identity file is read on the next refresh; do not invent a source boundary from manifest coverage. A temporarily unreadable invocation archive must not use a fixed continuation guess: block candidate UPSERTs from its valid coverage start onward, or from the retention lower bound when that start is absent; preserve materialized rows, and expose the existing error/retry state until its source becomes readable. A canonical day with no rows proves an empty replacement unless materialized dimensions have nonzero calls, Token, or cost; zero-total wall-time continuations remain valid. Queue mismatches durably, and replace a date only after a live/archive reconstruction matches both its daily and per-hour oracle totals.

## Guardrails / Reuse Notes

- Never use an aggregate row's `wall_time_ms` as a request count; preserve separate samples for every metric.
- Token and cost totals are sums of non-null samples, while performance metrics only accept successful rows with valid timings. Output speed is weighted by output tokens divided by stream duration, not an average of per-call speeds.
- Archive reads should enrich historical API Key identity from the soft-deleted account row. Missing or non-API-Key identities belong to the stable `other` series.
- API series keys must be issued by the bounded overview and validated against the same date window; reject more than eight keys before querying daily points.
- Removing a date from the full-rebuild replacement set is insufficient protection: remove that date from every partial and rebuilt candidate map before any UPSERT. Otherwise a partial candidate can still overwrite a retained complete row.
- A failed integrity repair must preserve existing rollups, or preserve an empty result when no durable row exists, keep the existing error state visible until its actionable queue entry is cleared, and retry every unprovable queued candidate with persisted bounded backoff. Persist a proof-revocation error before later source reads, so a later failure in the same refresh cannot restore `ready`. An all-zero terminal oracle, including a bucket containing only active calls, must trigger the same nonzero-dimension residue check as an empty canonical day. SQLite `BUSY/LOCKED` retries stay inside the low-priority long-term refresh path and must not redesign global writer behavior.
- When a new proof field is added to the canonical rollup, migrate and backfill it from the same live/archive invocation sources before enabling it as an integrity oracle. Default old rows to untrusted, preserve their operational aggregate when source comparison fails, and exclude that date from repair decisions until a full source comparison can mark it complete.
- Archive cleanup is a compare-and-swap protocol, not an ID-only delete: stage and finalization must require the same manifest identity, SHA-256, and `delete_pending` state. Finalize under an immediate SQLite transaction and re-hash the file. A legacy monthly writer must hold that same SQLite writer lock from pending-manifest reactivation through file replacement, so either cleanup removes the old identity before the write or the rewrite makes the cleanup attempt a no-op.

## References

- `src/long_term_stats.rs`
- `src/maintenance/archive/cleanup.rs`
- `docs/specs/5k89c-long-term-usage-analytics/SPEC.md`
