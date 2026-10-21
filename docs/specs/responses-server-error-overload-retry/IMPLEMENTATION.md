# Responses `server_error` 过载特征与同账号重试实现状态

> 当前有效需求以 `./SPEC.md` 为准；本文记录实现覆盖、验证和 rollout 事实。

## Current Status

- Implementation: 已在当前工作树实现
- Lifecycle: active
- Catalog note: 代码与本地回归测试已完成；本次交付以本地测试验证逻辑正确性，线上候选版本观察不作为前置 gate。

## Implementation Coverage

- `REQ-RSO-001`: `src/proxy/request_entry.rs` 新增 ASCII 小写、标点/空白归一和连续空格折叠，仅在 HTTP 200、`code=server_error` 且命中 canonical phrase 时返回 `SameAccountOnly`。
- `REQ-RSO-002`: `src/proxy/stream_gate.rs` 将 retry policy 传递到 SSE early gate 和 compact 首 chunk gate；首个业务事件或 body 已转发后仍沿用原 late-failure 路径。
- `REQ-RSO-003` and `REQ-RSO-007`: `src/proxy/failover.rs` 复用既有 4 次同账号预算、backoff 和累计 timeout；canonical `server_error` 命中后按当前累计 attempt 的剩余预算重试，不追加独立预算；canonical signal 之前的 group-429/413 仍使用各自既有规则，不被追溯改判或截断；新 policy 在预算耗尽时先保留 route failure/attempt 记录，再直接返回，不解析其他账号、route 或 forward-proxy channel；失败状态在 early gate 重试分支中先落入既有 terminal-error 保存路径。
- `REQ-RSO-004` and `REQ-RSO-005`: 既有 `server_is_overloaded`/并发限制分类保持 `LegacyFailover`；普通 `server_error`、模糊 `overloaded` 和非 HTTP 200 不进入新策略。
- `REQ-RSO-006`: metadata-prefixed SSE 与 compact early gate 的业务内容边界未改变，已向下游发送业务内容后不透明重试。
- `REQ-RSO-008`: 本次没有新增 API、SQLite schema、配置项或 UI 字段；只修改分类、gate、failover 和后端测试/主题文档。

## Verification Commands

- `cargo fmt --all -- --check`: passed locally after the final edits.
- The `routing_timeout_and_overload_failover` resource bucket passed in the Agent VM with 40 passed and 0 failed using `--test-threads=1`.
- The resource bucket covers classifier boundaries including internal punctuation/whitespace normalization, Responses SSE and compact same-account success, budget exhaustion without upstream switch, late-failure boundary, preceding-429 and mixed legacy/new overload sequences, mixed 413/429 budget caps, fixed forward-proxy channel selection through local HTTP CONNECT fixtures, and legacy route ladder regressions.
- `cargo check --locked --all-targets --all-features`: passed in the Agent VM with Rust 1.96.0.
- `cargo clippy --locked --all-targets --all-features -- -D warnings`: passed in the Agent VM.
- `cargo-nextest 0.9.148` 已安装在 Agent VM 的 Cargo bin 中；此前按共享 runner 执行 `bash .github/scripts/run-backend-tests.sh --profile stateful-sqlite` 时，profile 中仍有既有 SQLite/cache/启动/模型等待时序夹具失败，Responses retry 相关用例通过；此前同一 runner 的 11 个既有失败项已按完整失败清单重跑 11/11 passed。本次新增/修改逻辑的最终确定性验收以单线程 40/40 资源桶、check 和 Clippy 为准。
- Delivery acceptance: 主人已明确将本次计划的 `empirical_acceptance` 修正为 `not_required`；分类、gate、预算和路由行为以本地单元/集成测试作为逻辑正确性证据。
- Direct macOS test execution remained blocked by the known `libsqlx_macros` dynamic-library `mis-aligned LINKEDIT string pool` loader error; the passing assertions above came from the Agent VM.

## Rollout Facts

- The earlier dashboard observation of deployed `v4.9.9` is baseline evidence only; it is not evidence for this implementation.
- The current candidate has not been deployed from this task; this is not a delivery blocker after the explicit local-test acceptance decision.
- No database migration, public API compatibility change, configuration change or UI visual evidence is expected for this topic.

## Remaining Gaps

- 当前没有因本主题逻辑或验证结果产生的交付阻塞；本机 macOS 的 Rust 动态库加载问题由 Agent VM 验证环境承接，线上 empirical acceptance 按主人确认不作为本次交付 gate。

## References

- `./SPEC.md`
- `./HISTORY.md`
- `../../src/proxy/request_entry.rs`
- `../../src/proxy/stream_gate.rs`
- `../../src/proxy/failover.rs`
