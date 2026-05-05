# 代理热路径并发稳定性与传输背压收口 - Implementation

## Current State

- Canonical spec: `docs/specs/q8h3n-proxy-hot-path-streaming-stability/SPEC.md`
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Note: 已移除错误的 whole-proxy admission gate，`PROXY_REQUEST_CONCURRENCY_*` 已进入 deprecated/ignored 兼容态；共享测试机 `codex-testbox` 100 并行压测通过，确认 `/v1/*` 不再因本地 admission gate 返回 `503`。

## 验证

- `cargo fmt --check`
- `cargo check --tests`
- `cargo test proxy_request_tracking_can_reach_100_in_flight_without_local_rejection -- --nocapture`
- `cargo test proxy_openai_v1_via_pool_reads_request_body_without_local_admission_gate -- --nocapture`
- `cargo test list_invocations_ -- --nocapture`
- `scripts/shared-testbox-proxy-parallel-smoke`
