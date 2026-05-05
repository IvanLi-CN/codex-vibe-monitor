# 号池暂时无号时的 10 秒有界等待与 503 终态 - Implementation

## Current State

- Canonical spec: `docs/specs/667ae-pool-no-available-account-bounded-wait/SPEC.md`
- Implementation summary: 已实现，待 PR / CI / review-proof 收敛

## Migrated Implementation Notes

## 状态

- Status: 已实现，待 PR / CI / review-proof 收敛
- Created: 2026-04-04
- Last: 2026-04-04

## 非功能性验收 / 质量门槛（Quality Gates）

- `cargo fmt --check`
- `cargo check`
- `cargo test pool_route_waits_for_header_sticky_account_before_first_attempt -- --test-threads=1`
- `cargo test pool_route_body_sticky_returns_503_after_wait_timeout -- --test-threads=1`
- `cargo test pool_route_keeps_generic_no_candidate_when_other_accounts_are_unavailable_for_other_reasons -- --test-threads=1`
- `cargo test pool_route_returns_specific_ungrouped_error_when_all_candidates_are_ungrouped -- --test-threads=1`
- `cargo test pool_route_returns_ungrouped_error_for_sticky_account_when_cut_out_is_forbidden -- --test-threads=1`

## 文档更新（Docs to Update）

- `docs/specs/README.md`
