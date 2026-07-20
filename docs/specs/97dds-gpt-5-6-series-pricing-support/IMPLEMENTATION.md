# GPT-5.6 系列定价、缓存写入计费与模型入口支持 - Implementation

## Current State

- Status: completed
- Canonical spec: `docs/specs/97dds-gpt-5-6-series-pricing-support/SPEC.md`

## Delivery Checklist

- [x] Create the active topic spec and index entry.
- [x] Extend backend pricing models, API payloads, and SQLite persistence with explicit cache read/write pricing.
- [x] Refresh the repo-managed default pricing catalog and GPT-5.6 model fallback resolution.
- [x] Update cost estimation to use explicit cache read/write pricing when available without changing legacy-model semantics.
- [x] Add GPT-5.6 models to proxy presets, settings model lists, and `/v1/models` hijack payloads.
- [x] Split the Settings pricing UI into cache read and cache write columns and keep legacy payload ingestion coverage.
- [x] Generalize unsupported-model UI rendering away from the `gpt-5.5` special-case.
- [x] Persist cost buckets, derive cache-write Token counts, and expose total/model usage breakdown APIs.
- [x] Add accessible dashboard and upstream-account breakdown panels, cache hit rate in Token detail, plus `CW` invocation labels.
- [x] Preserve exact realtime cost buckets in mixed ranges and reconcile historical total costs through a dynamic `unknown` bucket.
- [x] Group usage detail rows by model and recorded reasoning effort, with an explicit unspecified fallback and cache-hit Token labelling.
- [x] Merge the dashboard and upstream-account cost/Token panels into one `Usage details` table with dual-line Token and amount cells plus a total column.
- [x] Retire CRS runtime configuration, polling, aggregation, retention, and API reads while keeping old SQLite tables untouched.
- [x] Extend records-side pricing observability with advisory `costAudit` totals and workflow-success usage audits that compare persisted cost against the current local catalog without rewriting historical truth.
- [x] Run Rust and web validation, capture visual evidence, and update this file with the final verification set.

## Verification

- `cargo fmt --check`
- `cargo check`
- `cargo test` (1520 passed, 45 ignored)
- `cargo test estimate_proxy_cost_breakdown_uses_explicit_gpt_5_6_sol_cache_read_and_write_prices`
- `cargo test ranged_summary_`
- `cargo test ranged_summary_groups_model_usage_by_reasoning_effort`
- `cd web && bun run test`
- `cd web && bun run test-storybook`
- `cd web && bun run build`
- `cd web && bun run build-storybook`
- `cd web && bun run demo:build`
- `bun run lint:web`
- The spec's `assets/` directory contains mock-only Storybook evidence for the shared desktop and 390px `Usage details` table. The 390px state verifies that no horizontal scrollbar is needed.
