# 模型管理与价格同步 实现状态

> 当前有效规范以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 事实。

## Current Status

- Implementation: 原模型管理能力与价格审核改进均已实现；定向后端、Web、Storybook、类型检查、lint 和构建验证通过。
- Lifecycle: active
- Catalog note: models.dev is fetched only on explicit preview; local estimates remain user-reviewed.
- Delivery: 主人已确认与锁定基线比较后的视觉候选；桌面、手机、短屏及零结果证据已更新为正式 Spec 资产。PR #1064 已创建，当前 PR head 的 CI 收敛与 Tier 3 正式审查待完成。

## Implementation Coverage

- `REQ-MODEL-MGMT-001` through `REQ-MODEL-MGMT-004`, `REQ-MODEL-MGMT-007`, `REQ-MODEL-MGMT-009`, and `REQ-MODEL-MGMT-010`: existing persistent union catalog, preset updates, deletion suppression, on-demand models.dev parsing, normalized supported prices, selected apply, and source links remain in `src/pricing.rs`, `src/schema.rs`, `src/app_state.rs`, `src/api/slices/model_management.rs`, and `web/src/pages/system/SystemModelsPage.tsx`.
- `REQ-MODEL-MGMT-005`, `REQ-MODEL-MGMT-006`, `REQ-MODEL-MGMT-008`, and `REQ-MODEL-MGMT-011`: `src/models_dev_sync_memory.rs` stores service-shared sparse provider selections, exact `(model ID, provider ID)` checkbox choices (including false), and explicit quote-provider choices. `GET/PATCH /api/settings/models/sync/state` exposes the state; the Web client restores it and queues optimistic changes for immediate serialized persistence with retry after failures.
- `REQ-MODEL-MGMT-012`: successful complete catalog previews establish a first-seen baseline and record subsequent exact model IDs; `ModelsDevSyncDialog.tsx` acknowledges discovery only after a row intersects its scroll viewport. The badge remains for the current review and clears on a later successful review after acknowledgment.
- `REQ-MODEL-MGMT-013`: the review compares normalized input, output, cache-read, cache-write, and reasoning prices while distinguishing zero from missing; changed prices have text and color cues, and missing local price is distinct from new-model discovery.
- `REQ-MODEL-MGMT-014`: the review has a 560px desktop minimum when viewport height allows, a viewport-bounded short-screen layout, an independently scrolling model region, and a provider picker outside dialog clipping. The picker stacks above the dialog, sizes to Radix's available viewport area, and scrolls its provider list independently. Closing the dialog restores focus to its trigger. The batch toolbar stays on one row on desktop and short desktop viewports, and uses a compact two-row layout on mobile while retaining full accessible labels.
- `REQ-MODEL-MGMT-015`: model select-all, invert, and select-none cover every eligible row in the current provider/status/model filters, including offscreen rows. Provider search and bulk actions operate on the provider result set.
- `REQ-MODEL-MGMT-016`: the catalog parser preserves optional lifecycle status; explicitly deprecated quotes are hidden by default and excluded from apply until shown. Missing or other statuses remain visible.
- `REQ-MODEL-MGMT-017`: unchanged quotes remain editable preferences but do not create writes. Apply counts show changed selected quotes, including those hidden by model search; the request contains only actual price changes.
- `web/src/features/system/models-dev-sync/selection.ts` contains pure selection and apply-range calculation; `web/src/features/system/models-dev-sync/ModelsDevSyncDialog.tsx` owns the review interaction. Preview requests are aborted and generation-guarded when a review closes or is replaced; the preview overlays local memory patches persisted after that request began, so a delayed snapshot cannot replace a newer saved choice. Closing is blocked while a price apply is in flight, including the footer cancel action, and unmounted dialogs ignore late apply results. `web/src/lib/api/`, `web/src/i18n/translations.ts`, and the system workspace stories/tests provide the typed API, localization, and regression surfaces.
- Existing apply and delete transactions still read the resulting price catalog before commit. A failed catalog read rolls back database changes and keeps the in-memory catalog and HTTP result consistent.

## Coverage / Rollout Summary

- Five additive SQLite tables are created idempotently during schema setup. They do not infer checkbox choices from existing prices or proxy presets and do not rewrite existing prices, presets, or historical invocation costs.
- The first successfully normalized complete catalog establishes the exact model-ID baseline and current provider selection set in a transaction. Later complete catalogs add newly discovered IDs without deleting absent history; retrieval or parsing failures do not advance discovery.
- Sparse user patches are serialized transactions. Independent keys coexist, same-key successful writes resolve by commit order, and viewed acknowledgments advance only existing discovery records.
- Earlier binaries ignore the additive sync-memory tables. Forward recovery can repeat table creation and catalog initialization without removing recorded selections or existing model/pricing data.

## Verification

- `bash .github/scripts/run-backend-tests.sh --profile stateful-sqlite`: rerun after the concurrency regression update; 1,384 passed, 1,609 skipped on codex-testbox. The updated case races provider deselection with viewed acknowledgment, races true/false writes to one model/provider key, and verifies the persisted result.
- `cargo test sync_memory -- --nocapture`: 4 passed after changing the GET path to use a read transaction. `cargo test models_dev_catalog_maps_supported_prices_and_reports_other_dimensions -- --nocapture`: 1 passed.
- `cargo check --locked --all-targets --all-features`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, and `bun run verify:rust`: passed on codex-testbox.
- Current repair candidate: `cd web && bun run test` passed (170 files, 1,716 passed, 6 skipped); `bun run typecheck:web`, `bun run lint:web`, and `cd web && bun run build` passed. Web lint reported 92 non-blocking warnings, and the production build reported the existing large-chunk advisory. The shared testbox did not have Bun installed, so the full Web unit suite ran locally with the repository's locked dependencies.
- Focused regression: the SystemWorkspace Storybook file passed 47 stories locally with Chromium, including delayed provider/model memory saves and the disabled cancel action during a delayed apply. `SystemModelsPage.test.tsx` and `selection.test.ts` passed 30 tests, including late-preview reconciliation after a successful memory save, apply-close protection, exact payload assertions, and all five price fields. The codex-testbox browser runner did not have its expected Chromium headless-shell build; the equivalent focused run passed locally. A full repository-wide Storybook attempt stopped after 61 passes when the browser connection closed in `DashboardWorkingConversationsSection`.
- Controlled Storybook evidence covers desktop and mobile in light and dark themes, a 500px short viewport in light and dark themes, and a filtered zero-visible-results state. Story-level viewport settings and play assertions verify the requested 1660x960, 393x852, and 1280x500 canvases. Same-path desktop and mobile comparisons against base `b68295595d875aead33ef31514eaf602279ece94` were reviewed and approved; canonical assets now include all seven controlled states. The updated focused Storybook file passed 47/47, `bun run typecheck:web` passed, and `git diff --check` passed after the toolbar refinement.
- The installed Spec drift checker passed with `Spec漂移=不存在`, and the installed `spec_contract_check.py` passed for `SPEC.md`.

## Remaining Delivery Gates

- Final Spec drift and contract checks, fresh current-head required CI, formal Tier 3 review, and merge.

## Related Changes

- [PR #1064](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1064) is open for convergence.

## References

- `./SPEC.md`
- `./HISTORY.md`
