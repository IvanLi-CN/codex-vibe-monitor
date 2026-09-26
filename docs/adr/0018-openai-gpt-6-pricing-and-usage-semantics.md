# ADR 0018: OpenAI GPT-6 pricing and usage semantics

## Status

Accepted

## Decision

OpenAI's official API pricing documentation is the authority for the GPT-6 model set and price snapshot. Sub2API's bundled LiteLLM mirror is a secondary source only; the checked snapshot contains no entries for `gpt-6-astra`, `gpt-6-sol`, or `gpt-6-luna`, so it cannot cross-check their numeric prices. The application will not import its full catalog or synchronize prices online. The repo-managed GPT-6 set is `gpt-6-astra`, `gpt-6-sol`, and `gpt-6-luna`. The pre-existing `gpt-6-terra` row remains compatibility-only and is excluded from official model discovery.

Read the exact cache-write Token count at `usage.input_tokens_details.cache_write_tokens`. When present, persist it separately and estimate ordinary input, cache reads, and cache writes as separate buckets. The invocation API adds nullable `reportedCacheWriteTokens`; existing `cacheWriteTokens` and aggregate Usage details retain their total non-cache input semantics. If the upstream field is absent, the existing `max(inputTokens - cacheReadTokens, 0)` cache-write inference remains for compatibility and must be represented as an estimate, not as upstream-reported usage. Reasoning Tokens use the output rate. Older live rows and archives remain NULL; no historical exact-count backfill is performed.

An absent or Standard actual billing tier uses Standard rates. OpenAI accepts request-level `fast` or `priority` for Fast mode and currently reports the actual Fast mode response tier as `priority`; that actual tier uses the known 2x multiplier. Explicit actual Batch, Flex, regional, or otherwise unsupported tiers produce unknown cost rather than silently falling back to Standard; request-only tier hints do not change the estimate.

Historical non-null invocation costs, user-edited pricing rows, and custom catalogs remain immutable during catalog updates. Model-specific request parameter capability validation remains upstream-owned; this project forwards the request and preserves upstream validation outcomes.

The schema change is additive: a nullable live invocation column is added before new writes use it, and new archives include the column while legacy archive reads project it as `NULL`. New retention source identities use a versioned digest; recovery accepts earlier v2 prepared-archive digests only when the reported cache-write value is `NULL` in both source and archive. Pricing-row DML remains separate from schema DDL. A stopped or partially completed startup is repaired forward through idempotent schema recognition and catalog seeding; deployed columns are never removed by program rollback.

## Consequences

The estimator may leave a cost unknown when billing metadata identifies a tier it cannot safely price. When an upstream omits cache-write usage, the resulting split is backward-compatible but may not match the true ordinary-input/cache-write distribution. The pricing catalog is intentionally a bounded application snapshot rather than a mirror of every upstream model or processing tier.
