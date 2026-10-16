# 模型管理与价格同步 实现状态

> 当前有效规范以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 事实。

## Current Status

- Implementation: 原模型管理能力与价格审核改进均已实现；LiteLLM 单一来源、SHA-256 报价版本、完整多维费率、独立报价持久化、兼容 `entries` 请求和事务应用均已接入。
- Lifecycle: active
- Catalog note: LiteLLM is fetched only on explicit preview; local estimates remain user-reviewed.
- Delivery: 本次实现从 `origin/main` 的 `th/litellm-price-catalog` 分支交付；视觉候选等待主人确认，正式审查按当前候选刷新。

## Implementation Coverage

- `REQ-MODEL-MGMT-019`: `src/proxy/request_entry.rs` serves local presets for exact `GET /v1/models` requests without recognized credentials before upstream URL construction, invocation allocation, or inference admission. A failed credential lookup also falls back to the public local catalog; valid pool route keys retain the existing dispatch behavior. Router regressions in `src/tests/stateful_sqlite/pricing_catalog_and_models_passthrough/public_models.rs` cover credential variants, legacy settings, empty catalogs, unavailable SQLite, and authentication on adjacent paths and inference endpoints.
- `REQ-MODEL-MGMT-001` through `REQ-MODEL-MGMT-004`, `REQ-MODEL-MGMT-007`, `REQ-MODEL-MGMT-009`, and `REQ-MODEL-MGMT-010`: existing persistent union catalog, preset updates, deletion suppression, on-demand LiteLLM parsing, normalized supported prices, selected apply, and source links remain in `src/pricing.rs`, `src/schema.rs`, `src/app_state.rs`, `src/api/slices/model_management.rs`, and `web/src/pages/system/SystemModelsPage.tsx`.
- `REQ-MODEL-MGMT-005`, `REQ-MODEL-MGMT-006`, `REQ-MODEL-MGMT-008`, and `REQ-MODEL-MGMT-011`: `src/models_dev_sync_memory.rs` stores service-shared sparse provider selections, exact `(model ID, provider ID)` checkbox choices (including false), and explicit quote-provider choices. `GET/PATCH /api/settings/models/sync/state` exposes the state; the Web client restores it and queues optimistic changes for immediate serialized persistence with retry after failures. A failed viewed acknowledgment remains unviewed in a later review until a save succeeds or the server reports it viewed.
- `REQ-MODEL-MGMT-012`: successful complete catalog previews establish a first-seen baseline and record subsequent exact model IDs; `ModelsDevSyncDialog.tsx` acknowledges discovery only after a row intersects its scroll viewport. The badge remains for the current review and clears on a later successful review after acknowledgment.
- `REQ-MODEL-MGMT-013`: the review compares normalized input, output, cache-read, cache-write, and reasoning prices while distinguishing zero from missing; changed prices have text and color cues, and missing local price is distinct from new-model discovery.
- `REQ-MODEL-MGMT-014`: the review has a 560px desktop minimum when viewport height allows, a viewport-bounded short-screen layout, an independently scrolling model region, and a provider picker outside dialog clipping. The picker stacks above the dialog, sizes to Radix's available viewport area, and scrolls its provider list independently. Closing the dialog restores focus to its trigger. The batch toolbar stays on one row on desktop and short desktop viewports, and uses a compact two-row layout on mobile while retaining full accessible labels.
- `REQ-MODEL-MGMT-015`: model select-all, invert, and select-none cover every eligible row in the current provider/status/model filters, including offscreen rows. Provider search and bulk actions operate on the provider result set.
- `REQ-MODEL-MGMT-016`: the catalog parser preserves optional lifecycle status; explicitly deprecated quotes are hidden by default and excluded from apply until shown. Missing or other statuses remain visible.
- `REQ-MODEL-MGMT-017`: unchanged quotes remain editable preferences but do not create writes. Apply counts show changed selected quotes, including those hidden by model search; the request contains only actual price changes.
- `REQ-MODEL-MGMT-018`: price-application failures use localized text, keep raw transport details out of the shared Alert, and offer an in-Alert retry for the current applicable selection.
- `REQ-MODEL-MGMT-020` through `REQ-MODEL-MGMT-022`: `src/api/slices/model_management.rs` reads the LiteLLM raw JSON, records a SHA-256 response version, normalizes complete rates into `PricingQuote`, and writes additive `pricing_model_quotes` rows. `quotes` is optional on settings and apply requests; omitted quotes preserve existing rows, while apply and delete update quote rows with the legacy projection atomically.
- `web/src/features/system/models-dev-sync/selection.ts` contains pure selection and apply-range calculation; `web/src/features/system/models-dev-sync/ModelsDevSyncDialog.tsx` owns the review interaction. Preview requests are aborted and generation-guarded when a review closes or is replaced; the preview overlays local memory patches persisted after that request began, so a delayed snapshot cannot replace a newer saved choice. Failed viewed acknowledgments do not clear the new-model indicator from the next review. Closing is blocked while a price apply is in flight, including the footer cancel action, and unmounted dialogs ignore late apply results. Preview-fetch, memory-save, and price-apply failures keep their retry buttons inside the shared Alert. `web/src/lib/api/`, `web/src/i18n/translations.ts`, and the system workspace stories/tests provide the typed API, localization, and regression surfaces.
- Existing apply and delete transactions still read the resulting price catalog before commit. A failed catalog read rolls back database changes and keeps the in-memory catalog and HTTP result consistent.

## Coverage / Rollout Summary

- Six additive SQLite tables are created idempotently during schema setup: five sync-memory tables and `pricing_model_quotes`. They do not infer checkbox choices from existing prices or proxy presets and do not rewrite existing prices, presets, or historical invocation costs.
- The first successfully normalized complete catalog establishes the exact model-ID baseline and current provider selection set in a transaction. Later complete catalogs add newly discovered IDs without deleting absent history; retrieval or parsing failures do not advance discovery.
- Sparse user patches are serialized transactions. Independent keys coexist, same-key successful writes resolve by commit order, and viewed acknowledgments advance only existing discovery records.
- Earlier binaries ignore the additive sync-memory tables. Forward recovery can repeat table creation and catalog initialization without removing recorded selections or existing model/pricing data.

## Verification

- Live acceptance for code candidate `62121e6eb261ba40a7c0a655bdf47eedf5387bc0`: the LiteLLM raw catalog was fetched anonymously at `2026-10-16T03:36:54Z` from `https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json` (`2,876,609` bytes, SHA-256 `83af88edfe2f22f65c7c57c4e91c0c0575d7dfd7510d4f837c96695272acc7bd`). The audit record in `assets/live-litellm-openai-coverage.json` compares 22 model IDs visible in the official [OpenAI pricing page](https://developers.openai.com/api/docs/pricing) against LiteLLM and records zero missing IDs. Both GPT Image 2.5 entries contain standard text input/cache at `$5/$1.25` and image input/cache/output at `$8/$2/$30` per million tokens, plus their Batch dimensions.
- Parser regression: `cargo test litellm_catalog -- --nocapture` passed. The fixtures verify quote-only image models, audio/video/reasoning token normalization, nested query and guardrail rates, per-text-message and other non-token units, tier names, context thresholds, threshold-only projection exclusion, and exclusion of LiteLLM's `sample_spec` placeholder.
- Persistence regression: `cargo test pricing_settings_api_preserves_quotes_when_legacy_request_omits_quotes -- --nocapture`, `cargo test quote_only_apply_restores_a_deleted_model_without_legacy_price -- --nocapture`, `cargo test litellm_apply_updates_only_selected_prices_and_marks_the_source -- --nocapture`, and `cargo test model_price_sync_and_delete_roll_back_if_catalog_snapshot_fails -- --nocapture` passed. These cover additive quote recovery, omitted-quote preservation, quote-only apply, selected-provider merging, explicit deletion, and transaction rollback.
- Backend quality gates: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and `cargo clippy --locked --all-targets --all-features -- -D warnings` passed locally. The full `stateful-sqlite` profile remains a CI gate for the current PR head.
- Web quality gates: `SystemModelsPage.test.tsx` and `selection.test.ts` passed 37/37 tests, and the Demo handler suite passed 36/36 tests; `bun run typecheck:web`, `bun run lint:web`, and `cd web && bun run build` passed. Web lint reported 93 non-blocking repository warnings, and the production build retained the existing large-chunk advisory.
- A new controlled mock-only visual candidate was captured after the quote-only row and detail-layout changes; canonical asset replacement remains pending owner confirmation. The current code candidate `62121e6e` was re-reviewed by all four read-only lanes, and the current PR CI plus Docs Pages checks pass. Remaining owner gates are the visual confirmation and Step 5C decision card.

## Remaining Delivery Gates

- Owner confirmation of the controlled visual candidate and the Step 5C release decision card. Merge remains outside this implementation turn and requires the owner's confirmation of the decision card and predicted version.

## References

- `./SPEC.md`
- `./HISTORY.md`
