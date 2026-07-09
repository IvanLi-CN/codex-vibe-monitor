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
- [x] Run Rust and web validation, capture visual evidence, and update this file with the final verification set.

## Verification

- `cargo check --quiet`
- `cargo test --quiet seed_default_pricing_catalog_`
- `cargo test --quiet ensure_schema_allows_opting_out_of_new_proxy_models_after_migration`
- `cargo test --quiet estimate_proxy_cost_`
- `cargo test --quiet pricing_settings_api_`
- `cargo test --quiet proxy_openai_v1_models_`
- `cd web && bun run test`
- `cd web && bun run test-storybook`
- Storybook local evidence captured from `Settings/SettingsPage` default story and stored at `docs/specs/97dds-gpt-5-6-series-pricing-support/assets/settings-pricing-cache-read-write-storybook.png`
