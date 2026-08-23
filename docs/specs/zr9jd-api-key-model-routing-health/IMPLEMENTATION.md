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
- Account-level success recovery compares parsed timestamps inside an immediate SQLite transaction and only clears a route failure proven to predate the request. New route failures and their derived cooldown deadlines persist the same millisecond precision; legacy second-precision rows in the request's same second fail closed. HTTP/WebSocket recovery now carries its real connect-attempt start instead of completion time. Failed terminals persist their account/model fence before releasing their combination reservation, so the release's availability signal cannot race a new selection. A failed persistence write or cancellation while that write is pending still releases the in-memory reservation but deliberately suppresses that signal. HTTP/WebSocket terminal cache observations may update model evidence independently, but global pool availability is published only while the account is active, enabled, not soft-deleted, and has no active route-failure fence.
- Queue-mode NoCandidate auditing preserves first-candidate wait semantics while taking a non-mutating capacity snapshot of the remaining eligible candidates, so conflict totals and reason counts cover the complete candidate set. Its `nextEligibleAt` is scoped to candidates excluded by this request's model cooldown rather than every route row for the model.
- The NoCandidate Storybook meta injects a deterministic locale through a non-persistent `I18nProvider`; production callers retain the existing persisted-locale default, while Story switching cannot mutate or inherit browser locale storage.
- The account-scoped model-routing read/reset endpoints expose the model state and reset contract. Structured account events include model, before/after state and priority, failure count and cooldown ETA. Event projections recover a missing request model from the linked upstream attempt or invocation for both account detail and global event-list reads.
- The account detail health/events tab renders mixed model states, cooldown ETA, failure summaries, recent event impact scope and a single-model reset action. Recent events omit request-model labels: route-transition events affect only that model, while generic account failures affect the entire account and omit empty route transitions. Direct health-tab routes wait for the selected account before hydrating recent actions.
- Storybook covers available, degraded, cooling, empty, read-only, error, reset interaction and impact-scope states; the model rows use one desktop column track and compact spacing, while failure context keeps the summary ahead of three aligned metadata fields and mobile stacks without horizontal overflow. Storybook canvas provides component evidence and mock-only `ui_demo` provides page-level desktop/mobile evidence.
- This delivery adds a normalized API Key-only route telemetry projection from persisted upstream attempts and model-route events, with one row for each real selection/retry and standalone unlinked state events. The global endpoint and a model-scoped 48-hour cursor endpoint deliberately exclude raw payloads, unnormalized diagnostics and account-pool grouping metadata.
- The timeline schema projects legacy Shanghai-local and RFC3339 timestamps into an indexed virtual UTC epoch. The production QueryBuilder uses that projection for time windows, cursors and ordering; a partial `attempt_id` latest-event index and a standalone-event epoch index keep the bounded reads free of correlated scans and temporary sort B-trees without changing records or pagination.
- The live page keeps the shared summary above four content-width tabs in the order `对话 / 最新记录 / 路由 / 代理`, defaults to routing, and starts the HTTP snapshot plus versioned `pool.model-routing-live` subscription only while that tab is active. No standalone model-routing route or main-navigation item exists. The route toolbar intentionally keeps only refresh and time-window controls; model and route-state filters remain available in the API contract but are not exposed on this page. One read-only Frappe Gantt instance owns the shared Beijing-time SVG axis, grid and task rows; model tasks divide the chart into first-level groups, and account-model tasks use the current API Key `display_name` (falling back to `API Key #id` only when absent). SVG-native state/priority bands plus milestone polygons project proportional intervals and controlled real recovery attempts onto those lanes. Each model task is also an accessible expansion control for that model's returned attempts, retries, and state events; each row can expose candidate comparison, reason, state/priority transition, HTTP result, latency, and invocation drilldown. Account-pool groups remain intentionally excluded from this surface. Login health becomes compact by default and each model row loads its own 48-hour evidence only when expanded.
- The mock-only `ui_demo` mirrors the route snapshot, route SSE topic and exact-model history endpoints with deterministic API Key fixtures. Its API Key fixtures use curated fictional display names, never production data, group labels, notes, or role suffixes; the page shell does not render a debug inspector. Page-level evidence therefore exercises the same snapshot, active subscription and lazy-history consumers as the product UI; Storybook remains the component-level coverage source.

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
- Owner-approved mock-only `ui_demo` page captures use the live page's routing tab at desktop 1440x900 and mobile 393x852 CSS viewports with element-first clipping of the tabs plus routing panel. The normalized PNGs are stored as `assets/model-routing-live-route-tab-desktop.png` and `assets/model-routing-live-route-tab-mobile.png`; the prior standalone-page captures are not used as current evidence.
- API Key HTTP, transport/stream (including exact-model failures before attempt creation), missing-model, 413, policy-toggle, background-sync, OAuth compatibility, sticky preservation, success/reset, and concurrency regressions: passed in the full Rust suite.
- `bun run check:bun-first` and `bun run lint:docs`: passed.
- `bun run lint:web`: passed with the repository's existing 86 warnings; no errors remain in the changed files.
- `spec_drift_check.sh --base-ref origin/main --spec-path docs/specs/zr9jd-api-key-model-routing-health/SPEC.md`: passed with no drift.
- Cache-hit protection state-machine, settings-contract and atomic reservation regression tests: passed. The reservation test races two candidate selections for a cap of one and admits exactly one request.
- Account recovery fence regressions cover a failure transaction committed after the request starts in the same second, legacy same-second ambiguity, exact millisecond SQLite persistence and reload boundaries, healthy-success no-op broadcasting, and shared terminal-observer handling of HTTP/WebSocket-shaped capture records that must not wake waiters while the account failure fence remains active. These observer fixtures do not replace full network HTTP/WebSocket end-to-end coverage.
- Account cooldown persistence coverage verifies the millisecond writer and exact runtime expiry boundary after reload, then replaces the value with a legacy second-precision RFC3339 timestamp and verifies the same parsed `DateTime` behavior.
- `PoolRoutingSettingsCard/CacheHitProtection` Storybook play coverage and the Settings/API Vitest coverage: passed. A local-only component capture confirmed the enabled control, 10% threshold and reroute selector without including it as a PR image asset.

## Delivery Status

- The explicit-model implementation and API Key temporary-failure scope update are complete. Final quality-gate results and delivery references are refreshed at merge-ready handoff.

## Related Changes

- None

## References

- `./SPEC.md`
- `./HISTORY.md`
