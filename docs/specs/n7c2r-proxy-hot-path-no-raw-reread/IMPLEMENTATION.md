# 代理热路径停止 response raw 二次回读 - Implementation

## Current State

- Canonical spec: `docs/specs/n7c2r-proxy-hot-path-no-raw-reread/SPEC.md`
- Implementation summary: 已实现，待 PR / CI 收敛

## Migrated Implementation Notes

## 状态

- Status: 已实现，待 PR / CI 收敛
- Note: `/v1/responses` 与 `/v1/responses/compact` 的 capture 热路径已改为只依赖 live stream parser 与 bounded preview；完整 raw 文件仍照常落盘，但请求处理阶段不再为判型或 metadata 补全回读 `response_raw_path`。

## 验证

- `cargo fmt --check`
- `cargo check`
- `cargo test proxy_capture_target_ -- --nocapture`
- `cargo test proxy_capture_target_large_stream_soak_keeps_rss_within_stable_window -- --ignored --nocapture --test-threads=1`

## Migrated Task-Ticket Sections

## Task Orchestration

- wave: 1
  - main-agent => 在 `src/main.rs` 中移除成功热路径对 `response_raw_path` 的 SSE hint / parse 回读，改为只使用 live parser 与 preview decode (skill: $fast-flow)
- wave: 2
  - main-agent => 保留 raw-file helper 作为非热路径能力，并新增测试计数器证明 proxy capture 热路径不再触发 raw fallback (skill: $fast-flow)
- wave: 3
  - main-agent => 更新 `src/tests/mod.rs`，覆盖 gzip 大流、超大终态 SSE、大非流 JSON、raw 截断等场景，并断言热路径 raw reread 次数为零 (skill: $fast-flow)
- wave: 4
  - main-agent => 执行本地验证、同步 spec 状态、创建 PR 并收敛到 merge-ready (skill: $plan-sync + $codex-review-loop + $fast-flow)
