# Responses `server_error` 过载特征与同账号重试实现状态

> 当前有效需求以 `./SPEC.md` 为准；本文记录实现覆盖、验证和 rollout 事实。

## Current Status

- Implementation: 已在当前工作树实现
- Lifecycle: active
- Catalog note: 代码与回归测试已完成；线上候选版本验收尚未完成，不能用此前 dashboard 观察替代。

## Implementation Coverage

- `REQ-RSO-001`: `src/proxy/request_entry.rs` 新增 ASCII 小写、标点/空白归一和连续空格折叠，仅在 HTTP 200、`code=server_error` 且命中 canonical phrase 时返回 `SameAccountOnly`。
- `REQ-RSO-002`: `src/proxy/stream_gate.rs` 将 retry policy 传递到 SSE early gate 和 compact 首 chunk gate；首个业务事件或 body 已转发后仍沿用原 late-failure 路径。
- `REQ-RSO-003` and `REQ-RSO-007`: `src/proxy/failover.rs` 复用既有 4 次同账号预算、backoff 和累计 timeout；新 policy 在预算耗尽时先保留 route failure/attempt 记录，再直接返回，不解析其他账号或 route。
- `REQ-RSO-004` and `REQ-RSO-005`: 既有 `server_is_overloaded`/并发限制分类保持 `LegacyFailover`；普通 `server_error`、模糊 `overloaded` 和非 HTTP 200 不进入新策略。
- `REQ-RSO-006`: metadata-prefixed SSE 与 compact early gate 的业务内容边界未改变，已向下游发送业务内容后不透明重试。
- `REQ-RSO-008`: 本次没有新增 API、SQLite schema、配置项或 UI 字段；只修改分类、gate、failover 和后端测试/主题文档。

## Verification Commands

- `cargo fmt --all -- --check`: passed locally after the final edits.
- `RUST_MIN_STACK=33554432 cargo test routing_timeout_and_overload_failover -- --nocapture`: passed in the Agent VM, 35 passed, 0 failed.
- The resource bucket covers classifier boundaries, Responses SSE and compact same-account success, budget exhaustion without upstream switch, late-failure boundary, mixed legacy/new overload sequences, and legacy route ladder regressions.
- `cargo check --locked --all-targets --all-features`: passed in the Agent VM with Rust 1.96.0.
- `cargo clippy --locked --all-targets --all-features -- -D warnings`: passed in the Agent VM.
- `bash .github/scripts/run-backend-tests.sh --profile stateful-sqlite`: blocked before execution because `cargo-nextest` is not installed in the validation VM.
- Direct macOS test execution remained blocked by the known `libsqlx_macros` dynamic-library `mis-aligned LINKEDIT string pool` loader error; the passing assertions above came from the Agent VM.

## Rollout Facts

- The earlier dashboard observation of deployed `v4.9.9` is baseline evidence only; it is not evidence for this implementation.
- The current candidate has not been deployed from this task, so empirical online acceptance is still pending.
- No database migration, public API compatibility change, configuration change or UI visual evidence is expected for this topic.

## Remaining Gaps

- Install or otherwise provide `cargo-nextest` before claiming the stateful-sqlite profile complete.
- Re-check an invocation after the candidate rollout and confirm multiple attempts with unchanged account and route; record the candidate version and scenario.

## References

- `./SPEC.md`
- `./HISTORY.md`
- `../../src/proxy/request_entry.rs`
- `../../src/proxy/stream_gate.rs`
- `../../src/proxy/failover.rs`
