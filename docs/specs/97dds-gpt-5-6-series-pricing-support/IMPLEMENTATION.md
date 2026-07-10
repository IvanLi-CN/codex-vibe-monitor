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
- [x] Add accessible dashboard and upstream-account breakdown panels plus `CW` invocation labels.
- [x] Preserve exact realtime cost buckets in mixed ranges and reconcile historical total costs through a dynamic `unknown` bucket.
- [x] Retire CRS runtime configuration, polling, aggregation, retention, and API reads while keeping old SQLite tables untouched.
- [x] Run Rust and web validation, capture visual evidence, and update this file with the final verification set.

## Verification

- `cargo fmt --check`
- `cargo check`
- `cargo test` (1517 passed, 45 ignored)
- `cargo test estimate_proxy_cost_breakdown_uses_explicit_gpt_5_6_sol_cache_read_and_write_prices`
- `cargo test ranged_summary_`
- `cd web && bun run test`
- `cd web && bun run test-storybook`
- `cd web && bun run build`
- `cd web && bun run lint`
- Storybook evidence is stored under this spec's `assets/` directory for Settings pricing, account cost detail, mobile Token detail, and mixed realtime/historical cost detail on desktop and mobile.
