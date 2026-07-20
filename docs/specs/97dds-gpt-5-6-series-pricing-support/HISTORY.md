# GPT-5.6 系列定价、缓存写入计费与模型入口支持 - History

## Key Decisions

- 2026-07-10: Created a dedicated topic spec because GPT-5.6 changes both the pricing contract and operator-facing model surfaces, which is larger than a one-off seed refresh.
- 2026-07-10: Locked the pricing truth source to the official OpenAI 2026-07-08 GPT-5.6 pricing release rather than inheriting `sub2api` fallback billing behavior.
- 2026-07-10: Chose additive compatibility for `cacheInputPer1m` so old payloads and existing saved rows continue to round-trip during the schema transition.
- 2026-07-10: Expose derived cache-write Token counts as a labelled billing-derived value (`max(inputTokens - cacheInputTokens, 0)`), never as a new upstream usage field; persist exact cost buckets only for new records.
- 2026-07-10: Replaced the mixed-range cost-breakdown veto with an additive `unknown` bucket. Records with complete persisted buckets keep their exact categories; records with only total cost contribute the full total to `unknown`; records without total cost contribute no cost.
- 2026-07-11: Changed usage-detail grouping from model-only to model-plus-recorded-reasoning-effort so the same model remains auditable across effort levels. Missing effort stays unspecified, and the Token detail labels the cache-read column as cache read without changing the billing field.
- 2026-07-12: Added a cache hit rate column to the Token breakdown. It divides the cache-read count by the row's cache write, cache-read, and output Token total, matching the dashboard Token KPI while remaining available for each model row.
- 2026-07-13: Unified dashboard and upstream-account cost and Token detail panels into one `Usage details` table. Cache write combines input and cache-write cost, output combines output and reasoning cost, total retains all known and historical `unknown` cost, and historical-only costs do not fabricate split amounts.
- 2026-07-20: Added records-side advisory cost auditing. Persisted invocation `cost` remains the truth source; `/api/invocations` may now compare it against a current-catalog local recomputation with a fixed `0.000001 USD` mismatch tolerance and explicit reasons for price-version drift or non-comparable history.
- 2026-07-20: Reused the same recorded/local cost semantics for workflow-success usage audits, while preserving the distinction between missing `reasoningTokens` and a real recorded `0`.
