# OpenAI GPT-5.6 与 GPT-6 定价、缓存计费和模型入口支持 - Implementation

## Current State

- Status: formal GPT-6 implementation, automated validation, and owner-approved mock visual evidence complete
- Canonical spec: `docs/specs/97dds-gpt-5-6-series-pricing-support/SPEC.md`

## Delivery Checklist

- [x] Create the active topic spec and index entry.
- [x] Extend backend pricing models, API payloads, and SQLite persistence with explicit cache read/write pricing.
- [x] Refresh the repo-managed default pricing catalog and GPT-5.6 model fallback resolution.
- [x] Refresh unchanged official GPT-5.6 Terra/Luna rows from the prior repo-managed catalog while preserving modified and custom catalog entries.
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
- [x] Add shared structured read-only model identity icons for the GPT-5.6 Sol/Terra/Luna family, including dated aliases, accessible IDs, invocation surfaces, usage/performance details, routing health, and long-term model summaries.
- [x] Extend the shared read-only identity renderer with product-selected general-purpose GPT-6 Astra/Sol/Luna MDI glyphs, calendar-valid dated aliases, theme colors, and standalone/embedded presentations; do not present these fallbacks as official OpenAI icons.
- [x] Keep GPT-6 chart legends on the product's compact identity pattern: series swatch, identity-colored 16px glyph, and reasoning label without a tile.
- [x] Group GPT-5.6 invocation-card model identity, reasoning effort, and FAST metadata in a reusable visual cluster while preserving legacy non-target and mismatch layouts.
- [x] Tighten the grouped Dashboard context cluster to a fixed 20px model segment, one tone-matched reasoning marker, 4px sibling spacing, no internal vertical separators, and omission of missing reasoning values while preserving FAST accessibility.
- [x] Record the initial temporary GPT-6 Sol/Terra/Luna seed using matching GPT-5.6 rates; the formal catalog migration below supersedes Sol/Luna with official pricing and retains Terra for compatibility only.
- [x] Validate calendar dates in dated model aliases and keep invalid or preview variants unpriced.
- [x] Restrict online proxy cost repair to null-cost terminal rows, keep existing persisted pricing fields immutable, refresh affected hourly rollups, and expose bounded progress in System Tasks.
- [x] Run Rust and web validation, capture visual evidence, and update this file with the final verification set.

## Formal GPT-6 Implementation

- [x] Advance the repo-managed catalog to `openai-standard-2026-09-23` with the official Astra/Sol/Luna Standard short- and long-context rates.
- [x] Refresh only unchanged temporary Sol/Luna seed rows, insert Astra idempotently, and preserve compatibility-only Terra, custom catalogs, edited rows, and persisted historical costs.
- [x] Add the official GPT-6 set to proxy presets, Settings/routing selection, structured model identity, and `/v1/models`; keep Terra out of official discovery.
- [x] Keep compatibility-only Terra out of hijacked `/v1/models` responses even when upstream model merging is enabled, without changing non-hijacked upstream passthrough.
- [x] Serialize the nullable reported-cache-write column migration with a SQLite write transaction and verify concurrent independent-pool re-entry.
- [x] Estimate exact ordinary-input/cache-read/cache-write buckets when the upstream reports cache-write usage, preserve inferred fallback when absent, and account for actual supported/unsupported service tiers and the long-context threshold.
- [x] Retain the most recently reported actual WebSocket response tier within a turn when a later usage event omits it, so unsupported tiers cannot fall back to Standard pricing.
- [x] Merge partial streaming cache-read/cache-write detail updates without erasing omitted aggregate usage fields.
- [x] Admit cache-only WebSocket `response.created`/`response.in_progress` usage into the turn accumulator without persisting a nonterminal row, and include it in interrupted-turn snapshots after first token.
- [x] Refresh richer usage from a later WebSocket terminal event in the existing invocation through the durable terminal writer; recompute affected hourly rollups while retaining terminal identity, status, and non-WebSocket duplicate behavior.
- [x] Rebuild hourly rollups after a WebSocket terminal insert race, and preserve legacy prepared-archive recovery with versioned source identities.
- [x] Finalize a published legacy prepared archive on retry using its journaled identity version and the verified artifact digest.
- [x] Resolve valid GPT-6 date aliases while leaving invalid and preview variants unpriced; do not add local model-specific request-parameter capability validation.
- [x] Restrict GPT-6 model identity icons to exact IDs and calendar-valid dated aliases; retain invalid dated aliases as original text.
- [x] Add targeted backend/web regression coverage.
- [x] Complete the stateful SQLite and archive resource profiles, Rust checks, and full Web unit/typecheck/build validation.
- [x] Obtain owner acceptance for current-only mock visual evidence and persist the canonical evidence set.

## Migration and Compatibility Record

- Durable state: SQLite `codex_invocations` rows and invocation archive datasets. The source state is the repository-managed `0.2.0` baseline schema/catalog and previously emitted archive schema; earlier same-Minor readers must ignore the additive nullable column, and the candidate must read prior schemas.
- Ordered operations: (1) add nullable `reported_cache_write_tokens` through an idempotent `BEGIN IMMEDIATE`-serialized `ensure_schema` column migration; (2) independently update only exact, unchanged temporary Sol/Luna catalog seed rows and promote the repo-managed catalog version; (3) write the exact field in new invocation rows and newly produced archives. Historical exact-count backfill is not applicable.
- Rollout and pause points: schema recognition is independently observable through SQLite column inspection; concurrent processes serialize schema inspection and additive DDL, and catalog migration is guarded by repo-managed version and exact previous row values. Startup may safely stop between schema and catalog operations; re-entry recognizes the column and retries the catalog migration.
- Recovery: deployed schema migrations are immutable. New retention source identities use v3; interrupted v2 journals from either pre-column or candidate column layouts remain verifiable, with the pre-column layout accepted only when the reported value is NULL in both live rows and archive. Re-preparing a compatible old journal promotes it to the current identity. Program rollback does not remove the column; a faulty release is repaired forward by a new program version. Backup restore remains separate disaster recovery.
- SemVer impact: separately recorded in `assets/version-impact-record.json`; public API `patch`, persistent state `minor`, release unit `minor`. Earlier readers can ignore the new live column, but an earlier release that re-archives invocation rows cannot preserve a column it does not know, so same-Minor write-forward compatibility is not guaranteed.
- Migration contract: separately recorded in `assets/persistent-state-migration-record.json`, including ordered additive DDL, guarded pricing seed DML, no historical usage backfill, legacy archive reads, and forward-only recovery.
- Validation: cover pre-column live databases, independent-pool concurrent migration, pre-column archives and prepared-journal recovery, new live/archive round-trips, idempotent re-entry, interrupted-startup forward repair, and candidate reads of prior schemas. Existing non-null historical costs remain unchanged.

## Verification

The checks below document the completed GPT-5.6 and temporary GPT-6 delivery only; formal GPT-6 verification is tracked separately below.

- `cargo fmt --check`
- `cargo check`
- `cargo test` (1856 passed, 45 ignored)
- `cargo test estimate_proxy_cost_breakdown_uses_explicit_gpt_5_6_sol_cache_read_and_write_prices`
- `cargo test seed_default_pricing_catalog_`
- `cargo test estimate_proxy_cost_falls_back_to_dated_gpt_5_6_terra_base_pricing`
- `cargo test estimate_proxy_cost_falls_back_to_dated_gpt_5_6_luna_base_pricing`
- `cargo test estimate_proxy_cost_uses_dated_gpt_6_default_pricing_presets`
- `cargo test startup_backfill_proxy_cost_audit_includes_catalog_and_cursor_detail`
- `cargo test backfill_proxy_missing_costs_skips_missing_model_or_usage_and_retries_unpriced_rows`
- `cargo test proxy_cost_backfill_versioned_progress_preserves_historical_rows`
- `bash .github/scripts/run-backend-tests.sh --profile stateful-sqlite`
- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- `cargo test ranged_summary_`
- `cargo test ranged_summary_groups_model_usage_by_reasoning_effort`
- `cd web && bun run test` (1313 passed, 6 skipped)
- `cd web && bun run test-storybook` (20 passed)
- `cd web && bun run build`
- `cd web && bun run build-storybook`
- `cd web && bun run test -- ModelIdentity.test.tsx UsageBreakdownTooltip.test.tsx ModelPerformanceTrigger.test.tsx InvocationTable.test.tsx`
- `cd web && bun run demo:build`
- `bun run lint:web`
- The spec's `assets/` directory contains mock-only Storybook evidence for the shared desktop and 390px `Usage details` table. The 390px state verifies that no horizontal scrollbar is needed.
- The spec's `assets/` directory contains owner-approved dark- and light-theme Storybook component captures for the tightened GPT-5.6 model context cluster. Mobile remains covered by deterministic E2E assertions and does not create a separate owner-facing evidence image.

## Formal GPT-6 Candidate Verification

- Passed: targeted Rust pricing, seed-upgrade, schema-upgrade, `/v1/models`, invocation API, legacy archive, and cache-write usage-summary tests.
- Passed: targeted Web unit tests for GPT-6 identity, invocation detail, available model options, and demo catalog fixtures (46 tests).
- Passed on the base-synchronized candidate: `stateful-sqlite` (1,320 passed, 1,454 skipped), `archive-file-io` (276 passed, 2,498 skipped), `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, Rust source-quality policy checks, and `cargo check --locked --all-targets --all-features`.
- Passed on the repair-batch-6 source snapshot: `stateful-sqlite` (1,326 passed, 1,455 skipped), `archive-file-io` (276 passed, 2,505 skipped), `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, and the Rust source-quality policy check.
- Final backend regressions verify partial nonterminal WebSocket cache usage in interrupted-turn snapshots, later richer same-identity terminal usage refreshing the invocation and hourly rollup, and unchanged duplicate idempotence. Earlier SSE/cache-detail regressions remain covered.
- Passed on the current unchanged Web sources: full Web unit suite (1,557 passed, 6 skipped), `bun run typecheck:web`, and Web production build. Build output includes existing stale Browserslist data and large-chunk warnings; no build errors. `bun run lint:web` previously passed with 85 warnings and no errors.
- On the rebased candidate, the `InvocationWorkflowDetailPanel` and `ModelIdentity` Storybook stories passed targeted interaction coverage (3 tests). A wider Storybook launch was unstable during Vite dependency optimization; the complete suite remains unverified. The Settings page interaction story remains manual because its existing page-wide axe audit reports unrelated accessibility violations. The complete mock-only Settings page is captured from `ui_demo`.
- Added regression coverage for exact cache-write cost splits on existing explicit-price rows, inconsistent reported counts, and preservation across later usage-bearing stream events including an explicit zero. `cargo test cache_write -- --nocapture` passed (9 tests).
- Review Repair Batch 8 rejects invalid-date and preview GPT-6 variants before custom exact-row lookup while retaining exact Terra compatibility pricing; reasoning-only stream event admission is explicitly deferred to a separate contract.
- Validation note: the first full Web unit run had one unrelated flaky assertion; its isolated rerun and the subsequent full-suite retry passed.
- Owner-approved and persisted: the rebased mock-only Settings selector and pricing table, exact cache-write invocation detail, and invalid dated GPT-6 identity capture. The current base has no same-path image baseline; the owner confirmed this current-only evidence set.
- Visual gate: `Storybook覆盖=通过` for the changed invocation detail and invalid-date identity fallback; Settings page interaction story remains manual due to pre-existing axe findings. `视觉证据目标源=ui_demo+storybook_canvas`; `视觉证据=存在`; `空白裁剪=已裁剪`; `聊天回图=已展示`; `证据落盘=已落盘`; `视觉比较=owner-confirmed` (`current-only` baseline). The evidence is mock-only and contains no live account data.
- Rebased candidate checks passed: `stateful-sqlite` (1,329 passed, 1,456 skipped), `archive-file-io` (276 passed, 2,509 skipped), Rust format/check/Clippy/source-quality policy, full Web unit suite (1,561 passed, 6 skipped), Web typecheck, and production build.
- Repair Batch 9 passed: `stateful-sqlite` (1,332 passed, 1,458 skipped), `archive-file-io` (277 passed, 2,513 skipped), Rust format/check/Clippy/source-quality policy, and the targeted cache-write-only WebSocket-to-API/rollup regression.
- Review repair validation passed: `stateful-sqlite` (1,336 passed, 1,462 skipped), `archive-file-io` (281 passed, 2,517 skipped), Rust fmt/check/Clippy/source-quality; targeted tests verify monotonic WebSocket refreshes and legacy prepared-archive finalization after its retry cursor is due.
- Pre-rebase candidate `743807ecebb253a77252e65ac98a578f00fab548` passed its recorded validations before main advanced; it is superseded by the base-synchronized source candidate below.
- Base-synchronized source candidate `0ea5b855990fbc29baf9d4a380831dfb1920f087`, based on `cdb7fbfa85e460e5fa666aa66834c9fc52745b8e`, passed: `stateful-sqlite` (1,338 passed, 1,461 skipped), `archive-file-io` (281 passed, 2,518 skipped), Rust fmt/check/Clippy/source-quality, Web unit tests (1,565 passed, 6 skipped), Web typecheck, and production build.
- GPT-6 pricing regressions cover unsupported actual `default` tier yielding unknown cost, missing and explicit `standard` tiers using Standard rates, request-only tier hints, and supported Fast pricing; GPT-5.6 tier behavior remains unchanged.
- Final review fixes are covered by targeted tests for unsupported actual WebSocket tier retention and upstream-merged Terra exclusion; non-hijacked upstream model passthrough remains unchanged.
- Base-synchronized source candidate `daf46a7b24ebff9bf79be09e6d36ee1845aade23`, based on `cdb7fbfa85e460e5fa666aa66834c9fc52745b8e`, passed: `lightweight` (1,134 passed, 1,666 skipped), `stateful-sqlite` (1,338 passed, 1,462 skipped), `archive-file-io` (281 passed, 2,519 skipped), Rust fmt/check/Clippy/source-quality, Web unit tests (1,565 passed, 6 skipped), Web typecheck, and production build. The targeted interrupted-WebSocket-tier regression also passed; Web build reported the existing stale Browserslist-data and large-chunk warnings.
- Pre-rebase repair candidate `eabac2d5` passed the listed validation on mainline base `cdb7fbfa85e460e5fa666aa66834c9fc52745b8e`; it is superseded by the base-synchronized candidate below.
- Base-synchronized repair source candidate `c51a5dd5c3903799c9a91a6f7a358ea86ef9e3bd`, based on current mainline `92dc13a688f5567ee1dc53a7c07cf3f5b697f586`, passed: `lightweight` (1,134 passed, 1,667 skipped), `stateful-sqlite` (1,339 passed, 1,462 skipped), `archive-file-io` (281 passed, 2,520 skipped; one existing leaky test), Rust fmt/check/Clippy/source-quality, Web unit tests (1,577 passed, 6 skipped), Web typecheck and production build, Spec contract/drift, and targeted Terra-variant and unsupported-tier refresh regressions. The repair preserves unsupported actual-tier unknown cost while retaining newly observed tier metadata. Visual render inputs for the approved GPT-6 Settings, invocation-usage, and invalid-date fallback captures are unchanged by the base rebase.
- Repair Batch 10 source candidate `53e207db584fdc38d9055d5a08719584cbc0513a` adds equality guards for stale WebSocket usage refreshes and rejects negative GPT-6 output counts as unknown cost. The two targeted regressions passed; the prior base-synchronized full profiles and current repair-specific checks remain the validation evidence for this narrow backend-only batch.
- Repair Batch 11 preserves known cost fields when a richer WebSocket usage refresh has no cost evidence, while unsupported actual tiers still replace cost with unknown. The targeted refresh regressions and Rust check/Clippy passed.
