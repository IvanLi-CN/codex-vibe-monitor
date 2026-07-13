# Proxy Usage Billing Breakdown

## Context

Proxy usage responses expose cache-read Token counts but not a separate cache-write value. Models with explicit cache-write pricing still need a reconciliable billing view across records, dashboard totals, and upstream-account activity.

## Decision

- Derive cache-write billing Tokens as `max(inputTokens - cacheInputTokens, 0)`.
- Keep `cacheInputTokens` as the upstream cache-read count; do not rename or overwrite it.
- Persist exact input, cache-write, cache-read, output, and reasoning cost buckets when a new terminal proxy record is priced.
- Treat cost buckets as immutable record-time facts. Historical records without complete buckets are never repriced: when total cost exists, the full record cost is accumulated as `unknown`; when total cost is absent, no cost is fabricated.

## Aggregation Contract

- The unified detail view presents non-overlapping cache write, cache-read, and output Token categories.
- Cache hit rate is `cache-read Tokens / (cache write + cache-read Tokens + output)` for each total or model row; a zero denominator is unavailable rather than reported as zero percent.
- Exact cost buckets retain input, cache write, cache read, output, reasoning, and `unknown` amounts for reconciliation.
- A record contributes either all five exact buckets or its full known total to `unknown`; partial buckets are not mixed with unknown for the same record.
- Total and model rows reconcile as `input + cache write + cache read + output + reasoning + unknown = total cost` within floating-point tolerance.
- Totals and model-plus-reasoning-effort rows share one accumulator. Model names fall back to `unknown`; missing or blank effort stays unspecified and is never inferred from model defaults.
- Detail tables list totals first, then useful model-plus-effort rows sorted by total Tokens and then total cost descending.

## UI Contract

- Invocation surfaces use `CW` for cache-write billing Tokens and `C` for cache-read Tokens, with accessible full labels.
- Dashboard and upstream-account cost and Token metric labels open the same `Usage details` table through hover, keyboard focus, and click.
- The fixed table order is model, cache write, cache read, cache hit rate, output, and total. Cache write combines input and cache-write amount; output combines output and reasoning amount; total includes every exact bucket and `unknown`.
- Cache write, cache read, output, and total cells render Token then amount. Cache hit rate renders its single value in the first line and reserves an empty second line to retain row alignment. The first cell presents model and effort on separate lines so the responsive table remains readable without an internal scrollbar.
- Historical rows that only contribute `unknown` retain their amount in total while their three split amount cells are unavailable. Rows without any known cost show all amounts as unavailable.
