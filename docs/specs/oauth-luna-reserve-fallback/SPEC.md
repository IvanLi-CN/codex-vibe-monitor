# OAuth Luna Reserve fallback

## Context and Scope

OAuth Codex usage responses can expose an additional structured limit for subscription accounts. The service needs to retain that signal as a separate quota capability and use it only under an explicit, operator-controlled fallback policy.

This topic covers the typed Reserve usage snapshot, additive SQLite persistence, OAuth account-pool routing, request observability, the automatic fallback setting, and the account-pool UI. It does not change ordinary quota accounting or make arbitrary `gpt-*-luna` models aliases for Reserve.

## Goals / Non-goals

### Goals

- Identify the upstream capability as **Luna Reserve**, its allowance as **Reserve Quota**, and its upstream wire model as `gpt-reserve`.
- Parse structured `additional_rate_limits` data in both object and array forms and retain a typed, freshness-aware Reserve snapshot.
- Route ordinary requests to Normal Quota first, then use Reserve only after a classified ordinary-quota exhaustion when automatic fallback is enabled.
- Keep explicit `gpt-reserve` requests Reserve-only and preserve the client-requested model identity in diagnostics.
- Expose Reserve availability, reported percentages, reset time, capture time, route, wire model, and fallback reason without fabricating an absolute balance.

### Non-goals

- Mapping every model containing `luna` to `gpt-reserve`.
- Advertising `gpt-reserve` in the ordinary public model catalog.
- Inferring Reserve availability from price, local token usage, or upsell text.
- Adding Reserve history charts or changing the ordinary quota window's statistics.

## Requirements

### MUST

- REQ-RESERVE-001: Missing, stale, or failed Reserve observations are `unknown`; they cannot authorize automatic fallback and must not be treated as `unavailable`.
- REQ-RESERVE-002: The automatic fallback switch defaults to enabled for OAuth routing, can be disabled globally, and does not disable usage synchronization or Reserve visibility.
- REQ-RESERVE-003: Automatic fallback requires a clear ordinary-quota exhaustion signal, refreshes usage at most once, and reuses the existing failover budget.
- REQ-RESERVE-004: Generic 429, 5xx, authentication, parameter, unsupported-model, and unknown failures do not trigger automatic Reserve fallback.
- REQ-RESERVE-005: A single Reserve failure does not mark Reserve exhausted without an explicit upstream signal or confirming usage snapshot.
- REQ-RESERVE-006: SQLite changes are additive and preserve existing databases and older usage snapshots.
- REQ-RESERVE-007: Account list/detail API responses distinguish `available`, `unavailable`, and `unknown` and do not expose or derive an absolute remaining balance.

## Functional Behavior

### Usage snapshot

The OAuth usage parser accepts `additional_rate_limits` as either a keyed object or an array of limit records. The Reserve record stores its window, reported percentage, reset time, availability state, source, and capture time. A snapshot is usable only while its structured data is fresh and explicitly available.

### Routing

Normal requests select Normal Quota first. After classified ordinary-quota exhaustion, the router may retry the current account with `gpt-reserve` and then select another Reserve-available OAuth account within the existing failover budget. The request's client model remains the original model; request diagnostics record `quota_route=reserve`, the Reserve wire model, and the fallback reason.

An explicit `gpt-reserve` request selects Reserve only and never downgrades to Normal Quota. Turning off automatic fallback prevents only the implicit route transition.

### Operator surfaces

Pool routing settings expose `automatic_reserve_fallback`. OAuth account summaries and details expose Reserve availability, usage percentage, available percentage when reported, reset time, and last capture time. `unknown` and `unavailable` have separate labels and visual states.

## Interfaces

| Interface                        | Change                                                   | Consumers                                           |
| -------------------------------- | -------------------------------------------------------- | --------------------------------------------------- |
| OAuth usage snapshot             | Add typed Reserve fields and object/array parsing        | usage sync, SQLite persistence, candidate selection |
| Pool routing settings API        | Add `automaticReserveFallback`                           | settings page, routing policy                       |
| OAuth account summary/detail API | Add Reserve status and reported window fields            | account list and detail views                       |
| Pool request diagnostics         | Add quota route, Reserve wire model, and fallback reason | invocation records and troubleshooting              |

## Verification

- VER-RESERVE-001: `cargo test reserve_usage_snapshot -- --nocapture` covers object and array parsing. covers: REQ-RESERVE-001, REQ-RESERVE-006
- VER-RESERVE-002: `cargo test reserve_routing -- --nocapture` covers fresh availability, ordinary-first routing, and the fallback switch. covers: REQ-RESERVE-001, REQ-RESERVE-002, REQ-RESERVE-003, REQ-RESERVE-004, REQ-RESERVE-005
- VER-RESERVE-003: `cargo test reserve_request_rewrite -- --nocapture` covers client model preservation and `gpt-reserve` wire rewriting. covers: REQ-RESERVE-003, REQ-RESERVE-004
- VER-RESERVE-004: `cargo fmt --all -- --check`, `cargo check --locked --all-targets --all-features`, and `cargo clippy --locked --all-targets --all-features -- -D warnings` pass locally. covers: REQ-RESERVE-006
- VER-RESERVE-005: `bash .github/scripts/run-backend-tests.sh --profile stateful-sqlite` passes all 1550 stateful tests plus the schema-template precheck. covers: REQ-RESERVE-001, REQ-RESERVE-006
- VER-RESERVE-006: `cd web && bun run test`, `bun run typecheck:web`, and `bun run build` pass; the Web App Demo provides account-list, account-detail, and settings-toggle visual evidence. covers: REQ-RESERVE-002, REQ-RESERVE-007

## Related ADRs

- [ADR 0051: OAuth Luna Reserve fallback routing](../../adr/0051-oauth-luna-reserve-fallback.md)
