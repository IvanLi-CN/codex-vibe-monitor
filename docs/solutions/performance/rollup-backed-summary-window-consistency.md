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
- Never treat `window.start >= retention_cutoff` as proof that live-only totals are complete.

## Guardrails / Reuse Notes

- When a summary window is expected to match a bucketed timeseries sum, add a regression test that compares both totals on a mixed archive/live fixture.
- Prefer the rollup-backed path for any window that can straddle archived and live days.
- If retention settings can change over time, assume older days may already exist only in rollup/archive even when the current cutoff no longer suggests it.

## References

- `docs/specs/9aucy-db-retention-archive/SPEC.md`
- `src/api/slices/prompt_cache_and_timeseries/summary_queries.rs`
- `src/tests/slices/pool_failover_window_h.rs`
