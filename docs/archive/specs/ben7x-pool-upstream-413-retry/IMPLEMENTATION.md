# 实现状态

## 当前状态

- Pool 上游 HTTP failure 路径已识别 `413 Payload Too Large`。
- live-first 首次上游 `413` 会记录失败，并在请求体可 replay 后进入原账号补试。
- replay / capture failover 主循环对上游 `413` 只允许同账号追加 1 次尝试；第二次仍失败后进入既有 distinct-account failover。
- distinct-account 预算耗尽时，若最后具体上游错误是 `413`，外部响应保持 `HTTP 413`。
- 上游 `413` 使用 `upstream_http_413` failure kind，与本地 `body_too_large` 分离。

## 验证

- `cargo fmt --check`
- `cargo check`
- `cargo test upstream_413 -- --test-threads=1`
- `cargo test preserves_413 -- --test-threads=1`
- `cargo test pool_route_returns_429_after_three_distinct_accounts_hit_upstream_429 -- --test-threads=1`
- `cargo test capture_target_pool_route_persists_attempt_rows_and_summary_fields -- --test-threads=1`
