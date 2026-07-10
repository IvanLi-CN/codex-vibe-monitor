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
  - docs/specs/z6ysw-dashboard-account-activity-tabs/SPEC.md
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
- When a top-level KPI and a visible breakdown explain the same live quantity, serve them from one backend activity snapshot with one `rangeEnd`, one runtime overlay read, and one aggregation algorithm. Do not let the top KPI use frontend timeseries math while the breakdown uses backend account aggregation.
- Prefer account-first aggregation for visible account breakdowns: calculate account rows, add an explicit unassigned bucket for traffic without an account, then derive summary rates and live counts by summing the rows. This keeps the visible decomposition able to explain the top number.
- Keep summary-only activity snapshots genuinely lightweight. A request that omits account rows should read bounded summary/read-model data plus the short active-tail rate window, not build and sort the full account preview/archive row set before dropping it from JSON.
- When a dashboard card combines a live main value with historical comparison rows, keep the semantic split explicit: the main value can come from a strict real-time read model, while comparison rows can continue to use a stable historical aggregate as long as they remain clearly secondary and do not overwrite the live truth source.
- If a page-level activity snapshot already includes the visible natural-day summary and rate window, consume that snapshot directly for the main card truth. Fetch only comparison windows separately instead of layering a second same-window summary subscription under the same visible panel.
- Batch visible local patches separately from head/snapshot reconcile. A 1 second visible patch batch is responsive enough for card updates while avoiding per-record rerenders.
- Put expensive HTTP reconcile and dense chart data commits behind a separate 5 second budget.
- For large aggregate endpoints that must keep their JSON shape, add conditional HTTP (`ETag` and `304 Not Modified`) instead of trimming fields.
- Add lightweight diagnostics counters for each path: visible patch count, head fetch count, SSE summary commit count, HTTP reconcile count, chart data commit count, and conditional fetch hit count.
- Keep `current` summary and dashboard account-activity reconcile on the same 5 second budget. A faster current-window reconcile without a matching backend live read model simply turns SQLite scan cost into a tighter request loop.
- When an endpoint still needs strict “currently in progress” truth, move that truth into a write-side live table or read model and let the 5 second reconcile read that bounded surface instead of rescanning the historical raw table.
- Treat dashboard working-set surfaces the same way: the 5-minute working-conversations head/count and snapshot pagination/count can both read a write-side bounded working-set table. Keep the response shape and main ordering stable, but accept `<=5s` bounded freshness instead of strict historical snapshot recomputation from the raw invocation table.
- Align write-side maintenance with the same freshness budget. If Dashboard accepts `<=5s` reconcile, request-tail derived writes that feed those read models can use short-window coalescing/batch flush, while terminal invocation and terminal attempt facts remain synchronous.
- For high-frequency `running` process state, prefer one shared in-process runtime store plus SSE/HTTP overlay over writing every progress snapshot into SQLite. Create the first minimal running shell as soon as the service admits a tracked proxy request, then enrich the same runtime key after body parse and upstream attempt progress. Records/current summary/current timeseries/account-activity should read DB terminal facts first and overlay only `running/pending` memory rows; terminal DB facts always win.
- Give every synthetic runtime snapshot an explicit lifecycle owner. For `pool-via-*` attempts, keep the snapshot visible while the downstream stream is active, then let the existing attempt cleanup guard remove any remaining non-terminal snapshot on every attempt terminal path, including success, failure, downstream disconnect, and task cancellation. Ordinary invocation terminal overlays remain owned by their separate persistence lifecycle.
- Do not apply activity-window or natural-day filters to strict current-live counters backed by memory `running/pending` overlay rows. A request that started earlier than the current 5-minute working set is still “current” for in-progress counts until terminal/tombstone; range totals, rate input rows, and recent previews must remain bounded by the selected reporting window.
- If terminal records are queued through a SQLite write controller, treat the immediate SSE terminal payload and runtime-store tombstone as the short-lived UI truth until the DB row catches up. HTTP reconcile must not interpret a temporarily missing terminal DB row as a deletion; it should preserve visible SSE state within the bounded freshness window.
- Prompt-cache working-set overlays must apply the same freshness predicate to terminal runtime tombstones that SQL applies to terminal DB rows. Keep `running/pending` runtime rows independent of the activity window, but exclude terminal runtime rows older than the selected working window before merging pages, counting `totalMatched`, or hydrating recent invocations.
- For future regressions, slow-path evidence needs to identify the class of work, not just that something was slow: emit endpoint/window/source-scope plus key counts or cache-hit state so operators can tell apart request-time scans, maintenance-time rebuilds, and cache hydration misses.

## Guardrails / Reuse Notes

- Do not delay KPI counters if the SSE payload is already authoritative for the selected window.
- Do not reuse a long-horizon aggregate endpoint as a shortcut for a real-time KPI when their semantic boundaries differ, even if the payload looks close enough.
- Do not reuse chart timeseries as the source of a current KPI when the same screen also shows a live breakdown. The chart can legitimately differ because it is trend history; the current KPI should come from the shared activity snapshot.
- Do not mount both a visible `dashboardActivity` snapshot consumer and a same-window `useSummary(window)` subscription for the same `today` / `yesterday` panel. That duplicates SSE listeners, open-resync, and refresh churn without improving owner-facing truth.
- Do not simplify chart visuals to solve render pressure; throttle the data commit feeding the chart instead.
- Keep timer constants exported when tests need to assert cadence without duplicating magic numbers.
- `304` handling must preserve the previous UI data and clear transient errors; it is a successful no-body response, not a failed fetch.
- Closed historical windows can commit immediately because they do not receive live churn.
- Large retained-history drawers need a separate history budget: load the first visible page only, fetch additional pages from the drawer scroll threshold, and let SSE refresh merge the already loaded range rather than replaying from page 1 to `total`.
- Virtualize dense invocation surfaces at the shared table component, and render only the active responsive layout. Hidden desktop/mobile duplicates still contribute DOM and event-handler pressure.
- Do not let HTTP open-resync depend on DB persistence of transient runtime rows. If DB writes are intentionally skipped for `running` snapshots, open-resync must overlay the same runtime store used by SSE across records, summary, timeseries, account activity, and prompt-cache working conversations; otherwise the UI will flicker or lose visible in-flight rows until terminal.
- Apply terminal-DB exclusion to every runtime overlay path, including stats summary live augmentation. A runtime memory row may outlive terminal persistence after a missed cleanup, so overlay code must query terminal DB keys and skip matching memory rows before counting in-progress totals, retry totals, or wait averages.
- Do not use a runtime-age cutoff to compensate for a synthetic snapshot lifecycle leak. An age filter hides the ownership defect and can remove a genuine long-running request from strict current-live counters; repair the terminal cleanup path instead.

## References

- `docs/specs/5932d-sse-proxy-live-sync/SPEC.md`
- `docs/specs/z6ysw-dashboard-account-activity-tabs/SPEC.md`
- `web/src/hooks/useStats.ts`
- `web/src/hooks/useDashboardWorkingConversations.ts`
- `web/src/hooks/useParallelWorkStats.ts`
- `web/src/components/DashboardActivityOverview.tsx`
- `web/src/components/PromptCacheConversationTable.tsx`
- `web/src/components/InvocationTable.tsx`
- `src/api/slices/prompt_cache_and_timeseries/timeseries.rs`
- `src/proxy/request_entry.rs`
