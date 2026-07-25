# API Key 上游按模型路由健康管理 实现状态（#zr9jd）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实。

## Current Status

- Implementation: complete
- Lifecycle: active
- Catalog note: model routing health for API Key upstream accounts

## Coverage / rollout summary

- `pool_upstream_account_model_routes` stores exact `account_id + model` state with seven-day retention, failure windows, cooldown ETA, last success/failure and last-seen timestamps.
- API Key model-specific errors are isolated in `failure_recording.rs`; OAuth, authentication, transport and generic upstream failures retain account-level behavior. Fresh and sticky candidate selection applies model demotion/exclusion without changing static model rules.
- `GET /model-routing` and `POST /model-routing/reset` expose the model state and reset contract. Structured account events include model, before/after state and priority, failure count and cooldown ETA. Event projections recover a missing request model from the linked upstream attempt or invocation for both account detail and global event-list reads.
- The account detail health/events tab renders mixed model states, cooldown ETA, failure summaries, recent event impact scope and a single-model reset action. Recent events omit request-model labels: route-transition events affect only that model, while generic account failures affect the entire account and omit empty route transitions. Direct health-tab routes wait for the selected account before hydrating recent actions.
- Storybook covers available, degraded, cooling, empty, read-only, error, reset interaction and impact-scope states; the model rows use one desktop column track and compact spacing, while failure context keeps the summary ahead of three aligned metadata fields and mobile stacks without horizontal overflow. Storybook canvas provides component evidence and mock-only `ui_demo` provides page-level desktop/mobile evidence.

## Implementation map

- Backend schema and maintenance: `src/schema.rs`, `src/upstream_accounts/core_schema_maintenance.rs`, `src/maintenance/retention.rs`, `src/maintenance/archive/{writers.rs}`.
- State machine and routing: `src/upstream_accounts/routing/model_health.rs`, `src/upstream_accounts/routing/failure_recording.rs`, `src/upstream_accounts/routing/selection.rs`.
- Attempt model propagation: `src/proxy/{request_entry.rs,dispatch.rs,failover.rs,websocket.rs,usage_persistence.rs}`.
- API and event projection: `src/upstream_accounts/{core_runtime_types.rs,core_models_rows.rs,crud_group_notes.rs,sync_account_imports_tags.rs,sync_group_sessions.rs}`, `src/maintenance/hourly_rollups.rs`.
- Web UI and demo: `web/src/features/account-pool/ModelRoutingHealthPanel.tsx`, its Storybook story, `web/src/pages/account-pool/UpstreamAccounts.page-local-shared.tsx`, `web/src/lib/api/core-upstream.ts`, `web/src/demo/handlers.ts`.

## Validation

- Targeted model routing state/reset and concurrent failure tests: passed.
- `cargo fmt --check`: passed.
- `cargo check`: passed.
- `RUST_MIN_STACK=33554432 cargo test`: 1683 passed, 0 failed, 45 ignored. The full run also covers the known deep-future stack test through the existing 32 MiB stack helper.
- `cd web && bun run test --run`: 131 files passed, 1285 passed, 6 skipped.
- `cd web && bun run test-storybook --run`: 8 files passed, 17 passed.
- `cd web && bun run build`: passed.
- Storybook canvas DOM checks: desktop model columns share identical tracks/left edges; mobile `scrollWidth` equals `clientWidth`.
- Mobile hardening checks: reset actions render at 44px touch height below `lg`, desktop remains 32px, and the panel exposes `dl/dt/dd` field semantics without changing grid tracks.
- The model-routing Storybook story enables its own color-contrast axe rule; light-theme metadata and cooldown text use the stronger local semantic tokens while the global palette debt remains unchanged.
- Storybook visual evidence: component-boundary desktop/mobile captures passed `require_margin` normalization.
- Storybook `MixedStates` play coverage: each failure context exposes one summary label and the upstream failure message once.
- Account-event request-model fallback stateful test and impact-scope RTL + Storybook interaction coverage: passed; the UI assertions also verify that request-model labels remain absent, informational events do not claim a failure impact, and recovery transitions do not claim an active model impact.
- Mock-only `ui_demo` desktop 1440x1000 capture and mobile 390x844 browser viewport (375x812 page capture): account/model impact scopes and route transition are visible without request-model labels or horizontal overflow.
- `bun run check:bun-first` and `bun run lint:docs`: passed.
- `bun run lint:web`: passed with the repository's existing 88 warnings and 1 informational diagnostic; no errors remain in the changed files.
- `spec_drift_check.sh --base-ref origin/main --spec-path docs/specs/zr9jd-api-key-model-routing-health/SPEC.md`: passed with no drift.

## Delivery Status

- The implementation and its quality gates are complete. Delivery references are recorded in the specification when applicable.

## Related Changes

- None

## References

- `./SPEC.md`
- `./HISTORY.md`
