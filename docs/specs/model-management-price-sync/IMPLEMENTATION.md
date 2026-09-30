# 模型管理与价格同步 实现状态

> 当前有效规范以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 事实。

## Current Status

- Implementation: 已实现并通过定向验证；视觉证据已获确认并归档。
- Lifecycle: active
- Catalog note: models.dev is fetched only on explicit preview; local estimates remain user-reviewed.

## Implementation Coverage

- `REQ-MODEL-MGMT-001` through `REQ-MODEL-MGMT-003`: `src/pricing.rs`, `src/schema.rs`, `src/app_state.rs`, `src/api/slices/model_management.rs`, and `src/tests/stateful_sqlite/pricing_catalog_and_models_passthrough/pricing_catalog.rs` implement the persistent union catalog, preset updates, deletion suppression, and historical-cost preservation.
- `REQ-MODEL-MGMT-004` through `REQ-MODEL-MGMT-010`: `src/api/slices/model_management.rs` implements on-demand models.dev parsing, supported token-price normalization, conflict candidates, selected apply, and source links; `web/src/pages/system/SystemModelsPage.tsx` implements the editable list and review dialog.
- `web/src/App.tsx`, `web/src/features/app-shell/navigation.ts`, `web/src/lib/api/`, `web/src/i18n/translations.ts`, and `web/src/pages/Settings.tsx` add the route, navigation, clients, translations, and remove duplicate legacy panels.
- Verification: stateful SQLite profile 1373 passed; Web unit suite 1663 passed and 6 skipped; focused models.dev parser tests 3 passed; model workspace Storybook stories 26 passed; Rust check, formatting, Web typecheck, and production build passed.
- Rollout facts: schema migration preserves static candidates and enabled membership once; durable suppressions keep deleted built-in rows absent after restart; explicit price re-add clears suppression and does not enable the preset.

## Coverage / rollout summary

- 迁移先创建持久模型目录，再以一次性版本标记导入旧预置候选和已有价格模型。
- 同步预览读取外部目录但不写本地状态；仅显式应用所选候选时写入价格和模型目录。

## Remaining Gaps

- No known feature implementation gaps remain. Formal review, CI convergence, and PR state are tracked by the active delivery flow.

## Related Changes

- None

## References

- `./SPEC.md`
- `./HISTORY.md`
