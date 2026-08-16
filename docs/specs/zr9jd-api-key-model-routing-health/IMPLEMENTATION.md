# API Key 上游按模型路由健康管理 实现状态（#zr9jd）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实。

## Current Status

- Implementation: complete
- Lifecycle: active
- Catalog note: model routing health for API Key upstream accounts

## Coverage / rollout summary

- `pool_upstream_account_model_routes` stores exact `account_id + model` state with seven-day retention, failure windows, cooldown ETA, last success/failure and last-seen timestamps.
- The same model-route record persists cache-hit protection state: dynamic concurrency and recovery limits, minimum-limit streak, cache cooldown ladder, last observed hit rate and expired-cooldown probe status. Cache observations are API Key-only and require success usage with at least 3840 input tokens; a strictly-low sample halves only future combination reservations, while healthy samples recover one slot at a time.
- `cacheHitProtection` extends global routing settings with disabled-by-default 10% threshold and `queue`/`reroute` overflow behavior. Disabling the feature or changing its threshold clears only cache-owned state; changing overflow mode preserves learned protection state. Cache cooldowns use 15/30/60 seconds, and every expired model cooldown, including non-cache failures, admits one controlled real business request before wider recovery.
- API Key model-specific errors and exact-model 5xx, 429, logical overload, transport, handshake, and stream failures are isolated in `failure_recording.rs` and use the same model state machine without account cooldown or sticky deletion. Missing-model, 413, other non-hard, and background-sync failures remain diagnostic-only. OAuth and explicit authentication/payment hard failures retain account-level behavior. Fresh and sticky candidate selection applies model demotion/exclusion without changing static model rules.
- `GET /model-routing` and `POST /model-routing/reset` expose the model state and reset contract. Structured account events include model, before/after state and priority, failure count and cooldown ETA. Event projections recover a missing request model from the linked upstream attempt or invocation for both account detail and global event-list reads.
- The account detail health/events tab renders mixed model states, cooldown ETA, failure summaries, recent event impact scope and a single-model reset action. Recent events omit request-model labels: route-transition events affect only that model, while generic account failures affect the entire account and omit empty route transitions. Direct health-tab routes wait for the selected account before hydrating recent actions.
- Storybook covers available, degraded, cooling, empty, read-only, error, reset interaction and impact-scope states; the model rows use one desktop column track and compact spacing, while failure context keeps the summary ahead of three aligned metadata fields and mobile stacks without horizontal overflow. Storybook canvas provides component evidence and mock-only `ui_demo` provides page-level desktop/mobile evidence.
- This delivery adds a normalized API Key-only route telemetry projection from persisted upstream attempts and model-route events, with one row for each real selection/retry and standalone unlinked state events. The global endpoint and a model-scoped 48-hour cursor endpoint deliberately exclude raw payloads and unnormalized diagnostics.
- The standalone model-routing page receives an HTTP snapshot and a versioned `pool.model-routing-live` subscription only while active. Model routing and live conversations are top-level siblings: Live retains `对话 / 最新记录 / 代理`, while `/model-routing` contains only routing state, attempt and account/invocation drill-down. Its model blocks render actual Recharts 24-hour Gantt charts: exact accounts are Y-axis lanes, observed state intervals are color bands, unknown history is explicit, and each real selection or retry is a separate marker. Login health becomes compact by default and each model row loads its own 48-hour evidence only when expanded.
- The mock-only `ui_demo` mirrors the route snapshot, route SSE topic and exact-model history endpoints with deterministic API Key fixtures. Page-level evidence therefore exercises the same snapshot, active subscription and lazy-history consumers as the product UI; Storybook remains the component-level coverage source.

## Implementation map

- Backend schema and maintenance: `src/schema.rs`, `src/upstream_accounts/core_schema_maintenance.rs`, `src/maintenance/retention.rs`, `src/maintenance/archive/{writers.rs}`.
- State machine and routing: `src/upstream_accounts/routing/{model_health.rs,failure_recording.rs,selection.rs,settings_runtime.rs}`.
- Attempt model propagation and combination reservations: `src/proxy/{payload_utils.rs,request_entry.rs,dispatch.rs,failover.rs,route_selection.rs,websocket.rs,usage_persistence.rs}`.
- API and event projection: `src/upstream_accounts/{core_runtime_types.rs,core_models_rows.rs,crud_group_notes.rs,sync_account_imports_tags.rs,sync_group_sessions.rs}`, `src/maintenance/hourly_rollups.rs`.
- Web UI and demo: `web/src/features/settings/PoolRoutingSettingsCard.tsx` and its `CacheHitProtection` Storybook story, `web/src/features/account-pool/ModelRoutingHealthPanel.tsx`, `web/src/features/live/ModelRoutingGantt.tsx`, `web/src/pages/account-pool/UpstreamAccounts.page-local-shared.tsx`, `web/src/lib/api/core-upstream.ts`, `web/src/demo/handlers.ts`.

## Validation

- Targeted model routing state/reset and concurrent failure tests: passed.
- `cargo fmt --check`: passed.
- `cargo check`: passed.
- `RUST_MIN_STACK=33554432 RUST_TEST_THREADS=1 cargo test`: 2197 passed, 0 failed, 45 ignored. The serialized run avoids the repository's unrelated shared-runtime timing races while still covering the full suite.
- `cd web && bun run test`: 143 files passed, 1404 passed, 6 skipped.
- `cd web && bun run test-storybook`: 21 files passed, 62 passed, 54 skipped.
- `cd web && bun run build`: passed.
- Storybook canvas DOM checks: desktop model columns share identical tracks/left edges; mobile `scrollWidth` equals `clientWidth`.
- Mobile hardening checks: reset actions render at 44px touch height below `lg`, desktop remains 32px, and the panel exposes `dl/dt/dd` field semantics without changing grid tracks.
- The model-routing Storybook story enables its own color-contrast axe rule; light-theme metadata and cooldown text use the stronger local semantic tokens while the global palette debt remains unchanged.
- Storybook visual evidence: component-boundary desktop/mobile captures passed `require_margin` normalization.
- Storybook `MixedStates` play coverage: each failure context exposes one summary label and the upstream failure message once.
- Account-event request-model fallback stateful test and impact-scope RTL + Storybook interaction coverage: passed; the UI assertions also verify that request-model labels remain absent, informational events do not claim a failure impact, and recovery transitions name their model without claiming an active impact.
- The mock-only demo includes an API Key HTTP 502 model degradation event for `gpt-5.6-terra`; desktop and mobile evidence show model scope without claiming the account or all models are affected.
- Invocation fallback correlation tolerates different production timestamp formats and reused invocation IDs by selecting the nearest matching invocation instead of requiring exact timestamp equality.
- Mock-only `ui_demo` page captures: desktop 1440x900 and mobile 393x852 route pages show all four tabs, model-first state, filters and expanded recovery evidence; account pages show the compact login summary and expanded exact-model 48-hour history. The browser's nested demo viewport is not captureable in this environment, so the same pure-local routes use controlled CDP viewport emulation and browser-viewport capture.
- API Key HTTP, transport/stream (including exact-model failures before attempt creation), missing-model, 413, policy-toggle, background-sync, OAuth compatibility, sticky preservation, success/reset, and concurrency regressions: passed in the full Rust suite.
- `bun run check:bun-first` and `bun run lint:docs`: passed.
- `bun run lint:web`: passed with the repository's existing 86 warnings; no errors remain in the changed files.
- `spec_drift_check.sh --base-ref origin/main --spec-path docs/specs/zr9jd-api-key-model-routing-health/SPEC.md`: passed with no drift.
- Cache-hit protection state-machine, settings-contract and atomic reservation regression tests: passed. The reservation test races two candidate selections for a cap of one and admits exactly one request.
- `PoolRoutingSettingsCard/CacheHitProtection` Storybook play coverage and the Settings/API Vitest coverage: passed. A local-only component capture confirmed the enabled control, 10% threshold and reroute selector without including it as a PR image asset.

## Delivery Status

- The explicit-model implementation and API Key temporary-failure scope update are complete. Final quality-gate results and delivery references are refreshed at merge-ready handoff.

## Related Changes

- None

## References

- `./SPEC.md`
- `./HISTORY.md`
