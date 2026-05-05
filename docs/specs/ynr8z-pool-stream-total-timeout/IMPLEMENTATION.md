# 号池流式上游误用整请求超时 - Implementation

## Current State

- Canonical spec: `docs/specs/ynr8z-pool-stream-total-timeout/SPEC.md`
- Implementation summary: 进行中

## Migrated Implementation Notes

## 状态

- Status: 进行中
- Created: 2026-03-17
- Last: 2026-03-17

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo test pool_openai_v1_e2e_stream_survives_short_request_timeout`
- `cargo test pool_openai_v1_responses_stream_survives_short_request_timeout`
- `cargo test proxy_openai_v1_e2e_stream_survives_short_request_timeout`

## 文档更新（Docs to Update）

- `docs/specs/README.md`：新增规格索引，并在流程推进后同步状态。
- `docs/specs/ynr8z-pool-stream-total-timeout/SPEC.md`：记录实现进展、验证与 PR/发布状态。
