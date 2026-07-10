# 后端测试资源分层模块化与运行时预算 实现状态（#q7yt7）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 已完成（测试树、profile-aware runner、三路 required checks 与 Stateful SQLite runtime budget 已完成服务端验证）
- Lifecycle: active
- Catalog note: 双 PR 主体与 runner-only runtime follow-up 均已完成

## Coverage / rollout summary

- 当前计划分为两个连续 PR：
  - PR1：深层测试模块化
  - PR2：profile-aware runner、required-check 拆分与 runtime budget 收口
- PR1 已将 `src/tests/slices` 与 `src/upstream_accounts/tests_part_*` 迁入真实模块树：
  - `src/tests/{lightweight,stateful_sqlite,archive_file_io}`
  - `src/upstream_accounts/tests/{lightweight,stateful_sqlite,archive_file_io}`
- PR1 已移除旧字母/编号切片文件名与 `src/upstream_accounts/tests/parts.rs` 聚合入口。
- 当前模块树仍通过最小必要的 `pub(crate)` helper 暴露跨文件测试支撑；PR2 不再扩展这类聚合面，只围绕 runner/CI/runtime 收口。
- PR2 已把 `.github/scripts/run-backend-tests.sh` 收口为 profile-aware runner，稳定入口固定为：
  - `--profile lightweight`
  - `--profile stateful-sqlite`
  - `--profile archive-file-io`
- PR2 已将不属于 `src/tests/**` / `src/upstream_accounts/tests/**` 的 136 个内联 backend unit tests 并回 `lightweight` profile，避免 profile split 造成 coverage 回归。
- PR2 已把 owner-facing backend required checks 更新为三个 job，并同步 `.github/quality-gates.json`、contract fixtures、release snapshot 自测与 live quality-gates fixtures。
- PR2 发现 `CI PR` 仅对 `base=main` 触发，导致 stacked PR 无服务端 checks；现已将 `CI PR` 的 `pull_request` 触发范围放开到所有 PR base，同时保留 `Label Gate` / `Review Policy` 与 live rules 对齐检查只对 `main` 生效。
- PR #576 已合并为 `main@405dfe7b8d4e44b33c25836528c936a9a6341704` 并发布为 `v2.21.1`；`CI Main` run `29072008929` 的三路 backend job 都通过。
- 该 CI Main 的 backend job wall time 为：
  - `lightweight`: `3m19s`
  - `stateful_sqlite`: `6m45s`
  - `archive_file_io`: `4m27s`
- Stateful SQLite 的 `6m45s` 比 `6m30s` 目标高 `15s`。完整本地 profile 在 4、6、8 nextest threads 下都通过；热执行时间分别为 `155.979s`、`102.461s`、`89.940s`。follow-up 固定为 6 threads，避免使用 8 threads 的更高资源放大。
- PR #579 已合并为 `main@8ceeb9bb097ea1f33e4ece9765d82a6b643d5652`。其 CI Main run `29074132864` 验证了 6-thread runner：
  - `lightweight`: `3m10s`
  - `stateful_sqlite`: `6m00s`
  - `archive_file_io`: `4m50s`
- 最慢的 Stateful SQLite job 为 `6m00s`，比 `6m30s` 预算低 `30s`，完成 runtime budget 收口。
- 本地 profile wall time（2026-07-09，热缓存）：
  - `lightweight`: 281 tests, `real 3.83s`
  - `stateful_sqlite`: 1040 tests, `real 66.97s`
  - `archive_file_io`: 195 tests, `real 29.14s`
- 本地 top offenders 采样：
  - `lightweight`: `raw_compression_budget_stops_after_first_batch_when_budget_is_exhausted` (`2.229s`)
  - `stateful_sqlite`: `pool_openai_v1_compact_overload_falls_back_to_alternate_route_before_body_forward` (`15.635s`)
  - `archive_file_io`: `send_pool_request_with_failover_returns_owner_unavailable_for_encrypted_session_lock` (`11.828s`)

## Remaining Gaps

- 本轮范围内无剩余缺口；后续运行时优化应作为独立主题，以新的 CI Main 基线重新评估。

## Related Changes

- PR #576: `refactor: modularize backend test trees by resource profile`
- PR #579: `perf(ci): bound stateful test concurrency`

## References

- `./SPEC.md`
- `./HISTORY.md`
