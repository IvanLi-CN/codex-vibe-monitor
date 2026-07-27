---
title: Rollup-backed summary windows must stay aligned with daily timeseries
module: web-dashboard
problem_type: correctness
component: stats summary queries
tags:
  - summary
  - rollup
  - retention
  - archive
status: active
related_specs:
  - docs/specs/9aucy-db-retention-archive/SPEC.md
  - docs/specs/gz5ns-dashboard-natural-day-kpi-semantics/SPEC.md
  - docs/specs/z6ysw-dashboard-account-activity-tabs/SPEC.md
---

# Rollup-backed summary window consistency

## Context

Natural-day summary windows such as `previous7d` can span both live rows and days that were already materialized into hourly rollups by an earlier retention setting.

## Symptoms

- `summary?window=previous7d` is smaller than the sum of `timeseries?range=7d&bucket=1d`.
- The gap appears only after data has been archived or materialized under a shorter prior retention window.

## Root Cause

The summary path short-circuited to live-only aggregation whenever the requested start looked newer than the current retention cutoff. That assumption is too weak: a range can still need rollup/archive reads even when its start is inside the current retention window.

## Resolution

- Keep natural-day summary reads on the same rollup-backed path as hourly timeseries.
- Use hourly rollups, full-hour live tail replay, and uncovered archive fallback together.
- Version coverage by repair generation. When a derived rollup schema or replay contract changes, invalidate the old generation's markers, clear only that generation's derived fields, and reset its live/archive progress in one transaction.
- Generation invalidation must also requeue completed source archives that were previously marked materialized. Deleting only the target replay marker is insufficient when the archive scheduler filters on the shared materialization timestamp.
- Materialize coverage per completed bucket only after every currently visible row in that bucket is at or behind the repair cursor. A caught-up global cursor or elapsed wall-clock span is not proof that each bucket was replayed.
- For rows that arrive in an already covered hour after its marker was written, merge an exact tail above `max(repair_cursor, bucket_recompute_watermark)` with the covered rollup baseline. Do not discard valid archive coverage or count rows already included by an authoritative bucket recomputation twice.
- Treat a completed full-bucket recomputation as authoritative through a per-bucket invocation watermark, and make an in-flight additive repair skip only rows at or below that watermark while still advancing its cursor. Do not turn the recomputation itself into a coverage marker: reads must keep using exact fallback until the independent repair cursor proves that the whole bucket is covered.
- Apply the same per-bucket watermark to every derived reader, including auxiliary non-success token tails. A reader that uses only the global repair cursor can double-count rows already included by a bucket recomputation.
- Read coverage markers, rollup rows, the repair cursor, and the exact live tail from one database snapshot. Separate pool reads can combine a pre-repair rollup with a post-repair cursor.
- Never treat `window.start >= retention_cutoff` as proof that live-only totals are complete.
- Keep account-scoped summary variants aligned with the non-account summary path when they
  split work between exact live reads and rollup-backed tails.
- For windows that have no completed full hour yet, do not clamp exact live reads to the
  rollup live cursor or add a second tail replay path just for account-scoped summaries.

## Guardrails / Reuse Notes

- When a summary window is expected to match a bucketed timeseries sum, add a regression test that compares both totals on a mixed archive/live fixture.
- Prefer the rollup-backed path for any window that can straddle archived and live days.
- If retention settings can change over time, assume older days may already exist only in rollup/archive even when the current cutoff no longer suggests it.
- When adding an account-scoped summary variant, compare its no-full-hour behavior against the
  non-account path before introducing cursor-specific tail handling.
- Make generation initialization idempotent and transactional so repeated service restarts cannot preserve false markers, double replay derived fields, or mutate raw invocations and unrelated rollups.
- Advance live coverage by checking only buckets touched by the current repair batch. Re-running a full-table grouped scan after every small batch makes upgrade repair quadratic and unnecessarily prolongs exact fallback.
- Aggregate covered-hour live tails in one set-based query with per-bucket thresholds. Running the full account aggregation once per covered hour creates an avoidable N+1 path for seven-day Dashboard snapshots.
- Keep missing archive material explicitly uncovered. Exact fallback may use readable archive batches, but unavailable source rows must never be represented by a synthetic coverage marker.

## References

- `docs/specs/9aucy-db-retention-archive/SPEC.md`
- `src/api/slices/prompt_cache_and_timeseries/summary_queries.rs`
- `src/tests/slices/pool_failover_window_h.rs`
