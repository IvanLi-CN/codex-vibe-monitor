# Proxy Usage Billing Breakdown

## Context

Proxy usage responses expose cache-read Token counts but not a separate cache-write value. Models with explicit cache-write pricing still need a reconciliable billing view across records, dashboard totals, and upstream-account activity.

## Decision

- Derive cache-write billing Tokens as `max(inputTokens - cacheInputTokens, 0)`.
- Keep `cacheInputTokens` as the upstream cache-read count; do not rename or overwrite it.
- Persist exact input, cache-write, cache-read, output, and reasoning cost buckets when a new terminal proxy record is priced.
- Treat cost buckets as immutable record-time facts. Historical records without them expose total cost only and mark detailed cost unavailable.

## Aggregation Contract

- Token detail presents cache write, cache read, and output so the categories do not overlap.
- Cost detail presents input, cache write, cache read, output, and non-zero reasoning amounts.
- Totals and model rows share one accumulator. Model names fall back to `unknown` so every contribution remains auditable.
- Detail panels list totals first, then useful model rows sorted by the panel metric descending.

## UI Contract

- Invocation surfaces use `CW` for cache-write billing Tokens and `C` for cache-read Tokens, with accessible full labels.
- Dashboard and upstream-account metric labels open the same detail through hover, keyboard focus, and click.
- Narrow layouts constrain the popover width and keep the detail body scrollable instead of truncating values.
