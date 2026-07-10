# Proxy Usage Billing Breakdown

## Context

Proxy usage responses expose cache-read Token counts but not a separate cache-write value. Models with explicit cache-write pricing still need a reconciliable billing view across records, dashboard totals, and upstream-account activity.

## Decision

- Derive cache-write billing Tokens as `max(inputTokens - cacheInputTokens, 0)`.
- Keep `cacheInputTokens` as the upstream cache-read count; do not rename or overwrite it.
- Persist exact input, cache-write, cache-read, output, and reasoning cost buckets when a new terminal proxy record is priced.
- Treat cost buckets as immutable record-time facts. Historical records without complete buckets are never repriced: when total cost exists, the full record cost is accumulated as `unknown`; when total cost is absent, no cost is fabricated.

## Aggregation Contract

- Token detail presents cache write, cache read, and output so the categories do not overlap.
- Cost detail presents input, cache write, cache read, output, reasoning, and a dynamic unknown amount.
- A record contributes either all five exact buckets or its full known total to `unknown`; partial buckets are not mixed with unknown for the same record.
- Total and model rows reconcile as `input + cache write + cache read + output + reasoning + unknown = total cost` within floating-point tolerance.
- Totals and model rows share one accumulator. Model names fall back to `unknown` so every contribution remains auditable.
- Detail panels list totals first, then useful model rows sorted by the panel metric descending.

## UI Contract

- Invocation surfaces use `CW` for cache-write billing Tokens and `C` for cache-read Tokens, with accessible full labels.
- Dashboard and upstream-account metric labels open the same detail through hover, keyboard focus, and click.
- Breakdown panels use one semantic horizontal table with totals first and one model per row; responsive widths keep the table readable without an internal scrollbar.
- The unknown column appears only when the total or at least one model has a non-zero unknown amount, so exact-only ranges retain the five-column cost layout.
