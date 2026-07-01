---
title: Realtime dashboard reconcile budget
module: web-dashboard
problem_type: performance
component: React dashboard hooks
tags:
  - dashboard
  - sse
  - throttle
  - conditional-http
status: active
related_specs:
  - docs/specs/5932d-sse-proxy-live-sync/SPEC.md
---

# Realtime dashboard reconcile budget

## Context

Dashboard surfaces often consume the same SSE `records` stream for several different jobs: KPI counters, dense charts, working conversation cards, and heavier aggregate sections. Treating every SSE record as permission to refetch and rerender every surface creates avoidable CPU and network load.

## Symptoms

- KPI numbers need to feel live, but charts and aggregate sections churn on every record.
- Working conversation cards repaint repeatedly while a burst of records belongs to the same visible conversation.
- Large aggregate payloads such as `parallel-work` are requested frequently even when the response body is unchanged.

## Root Cause

The stream mixes three update classes with different budgets:

- visible lightweight state that can be patched locally,
- authoritative HTTP reconcile that can lag by a few seconds,
- large aggregate payloads that often do not change between adjacent reconciles.

Using one cadence for all three overfits the most urgent surface and overloads the rest.

## Resolution

- Let SSE summary payloads drive KPI-style counters directly when the payload already contains the authoritative window.
- Keep lightweight live KPI semantics separate from heavy aggregate endpoint semantics. If the KPI means “strictly in progress now”, expose that directly on the summary path instead of reusing the latest point from a historical bucket series.
- When a dashboard card combines a live main value with historical comparison rows, keep the semantic split explicit: the main value can come from a strict real-time read model, while comparison rows can continue to use a stable historical aggregate as long as they remain clearly secondary and do not overwrite the live truth source.
- Batch visible local patches separately from head/snapshot reconcile. A 1 second visible patch batch is responsive enough for card updates while avoiding per-record rerenders.
- Put expensive HTTP reconcile and dense chart data commits behind a separate 5 second budget.
- For large aggregate endpoints that must keep their JSON shape, add conditional HTTP (`ETag` and `304 Not Modified`) instead of trimming fields.
- Add lightweight diagnostics counters for each path: visible patch count, head fetch count, SSE summary commit count, HTTP reconcile count, chart data commit count, and conditional fetch hit count.
- Keep `current` summary and dashboard account-activity reconcile on the same 5 second budget. A faster current-window reconcile without a matching backend live read model simply turns SQLite scan cost into a tighter request loop.
- When an endpoint still needs strict “currently in progress” truth, move that truth into a write-side live table or read model and let the 5 second reconcile read that bounded surface instead of rescanning the historical raw table.
- Treat dashboard working-set surfaces the same way: the 5-minute working-conversations head/count and snapshot pagination/count can both read a write-side bounded working-set table. Keep the response shape and main ordering stable, but accept `<=5s` bounded freshness instead of strict historical snapshot recomputation from the raw invocation table.
- Align write-side maintenance with the same freshness budget. If Dashboard accepts `<=5s` reconcile, request-tail derived writes that feed those read models can use short-window coalescing/batch flush, while terminal invocation and terminal attempt facts remain synchronous.
- For future regressions, slow-path evidence needs to identify the class of work, not just that something was slow: emit endpoint/window/source-scope plus key counts or cache-hit state so operators can tell apart request-time scans, maintenance-time rebuilds, and cache hydration misses.

## Guardrails / Reuse Notes

- Do not delay KPI counters if the SSE payload is already authoritative for the selected window.
- Do not reuse a long-horizon aggregate endpoint as a shortcut for a real-time KPI when their semantic boundaries differ, even if the payload looks close enough.
- Do not simplify chart visuals to solve render pressure; throttle the data commit feeding the chart instead.
- Keep timer constants exported when tests need to assert cadence without duplicating magic numbers.
- `304` handling must preserve the previous UI data and clear transient errors; it is a successful no-body response, not a failed fetch.
- Closed historical windows can commit immediately because they do not receive live churn.
- Large retained-history drawers need a separate history budget: load the first visible page only, fetch additional pages from the drawer scroll threshold, and let SSE refresh merge the already loaded range rather than replaying from page 1 to `total`.
- Virtualize dense invocation surfaces at the shared table component, and render only the active responsive layout. Hidden desktop/mobile duplicates still contribute DOM and event-handler pressure.

## References

- `docs/specs/5932d-sse-proxy-live-sync/SPEC.md`
- `web/src/hooks/useStats.ts`
- `web/src/hooks/useDashboardWorkingConversations.ts`
- `web/src/hooks/useParallelWorkStats.ts`
- `web/src/components/DashboardActivityOverview.tsx`
- `web/src/components/PromptCacheConversationTable.tsx`
- `web/src/components/InvocationTable.tsx`
- `src/api/slices/prompt_cache_and_timeseries/timeseries.rs`
