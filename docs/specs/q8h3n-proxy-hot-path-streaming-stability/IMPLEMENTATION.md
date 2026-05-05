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

## Migrated Task-Ticket Sections

## Task Orchestration

- wave: 1
  - main-agent => 从 `/v1/*` 入口移除 whole-proxy admission gate，把 permit 改成纯观测型 in-flight tracking，并对 `PROXY_REQUEST_CONCURRENCY_*` 输出 deprecated/ignored 告警 (skill: $fast-flow)
- wave: 2
  - main-agent => 保留并复验 request/response raw 异步旁路与 summary/quota debounce，确保移除 gate 后业务流优先级不回退 (skill: $fast-flow)
- wave: 3
  - main-agent => 补齐本地回归：100 并行不再本地 503、长流期间 in-flight tracking 到 body 结束、raw 饱和只丢 capture、不影响业务流 (skill: $fast-flow)
- wave: 4
  - main-agent => 新增 `scripts/shared-testbox-proxy-parallel-smoke`，在 `codex-testbox` 上用隔离 run dir、唯一 Compose project、LXC caps 兼容 override 跑 100 并行压测 (skill: $shared-testbox-runner)
- wave: 5
  - main-agent => 完成本地验证、共享测试机压测、fast-track review/PR/merge/release，并在 101 与浏览器侧复验不再出现本地 admission reject (skill: $fast-flow)
