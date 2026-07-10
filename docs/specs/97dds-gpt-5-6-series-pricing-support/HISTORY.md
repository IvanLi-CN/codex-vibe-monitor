# GPT-5.6 系列定价、缓存写入计费与模型入口支持 - History

## Key Decisions

- 2026-07-10: Created a dedicated topic spec because GPT-5.6 changes both the pricing contract and operator-facing model surfaces, which is larger than a one-off seed refresh.
- 2026-07-10: Locked the pricing truth source to the official OpenAI 2026-07-08 GPT-5.6 pricing release rather than inheriting `sub2api` fallback billing behavior.
- 2026-07-10: Chose additive compatibility for `cacheInputPer1m` so old payloads and existing saved rows continue to round-trip during the schema transition.
- 2026-07-10: Expose derived cache-write Token counts as a labelled billing-derived value (`max(inputTokens - cacheInputTokens, 0)`), never as a new upstream usage field; persist exact cost buckets only for new records.
- 2026-07-10: Replaced the mixed-range cost-breakdown veto with an additive `unknown` bucket. Records with complete persisted buckets keep their exact categories; records with only total cost contribute the full total to `unknown`; records without total cost contribute no cost.
