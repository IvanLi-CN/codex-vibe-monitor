# 号池 `/v1/responses*` 临时失败改为“每个当前账号先重试再切号” - Implementation

## Current State

- Canonical spec: `docs/specs/p6x4r-pool-responses-per-account-retry-budget/SPEC.md`
- Implementation summary: 已实现，待 PR / CI / review-proof 收敛

## Migrated Implementation Notes

## 状态

- Status: 已实现，待 PR / CI / review-proof 收敛
- Created: 2026-04-15
- Last: 2026-04-15
- Note: fast-track / pool `/v1/responses*` temporary failure family only / no HTTP API or SQLite schema change

## 验证

- `cargo fmt --check`
- `cargo check --tests`
- `cargo test pool_same_account_attempt_budget_keeps_follow_up_accounts_retryable_for_responses_family -- --nocapture`
- `cargo test pool_route_responses_compact_retries_follow_up_accounts_before_switching -- --nocapture`
- `cargo test capture_target_pool_route_stops_after_three_distinct_accounts -- --nocapture`
- `cargo test capture_target_pool_route_timeout_switches_to_alternate_upstream_route -- --nocapture`
- `cargo test capture_target_pool_route_timeout_returns_no_alternate_when_only_same_route_remains -- --nocapture`
- `cargo test capture_target_pool_route_timeout_surfaces_blocked_policy_terminal -- --nocapture`
- `cargo test capture_target_pool_route_timeout_ignores_broken_same_route_groups -- --nocapture`
- `cargo test pool_route_existing_sticky_owner_preserves_last_failure_after_cutout_alternate_fails -- --nocapture`
- `cargo test pool_route_existing_sticky_owner_preserves_last_failure_after_distinct_budget_exhausts -- --nocapture`
- `cargo test pool_route_does_not_use_pool_wide_429_message_when_budget_exhaustion_is_mixed -- --nocapture`
- `cargo test pool_route_group_upstream_429_retry_keeps_separate_budget_from_server_errors -- --nocapture`
- `cargo test pool_route_live_request_switches_accounts_immediately_after_upstream_429 -- --nocapture`
- `cargo test pool_openai_v1_responses_failover_reapplies_account_fast_mode_from_original_body -- --nocapture`
- `cargo test pool_openai_v1_responses_compact_total_timeout_caps_same_account_retry_before_first_byte -- --nocapture`
- `cargo test pool_route_compact_502_returns_cvm_id_and_attempt_observations -- --nocapture`
