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
