# 后端测试资源分层模块化与运行时预算 实现状态（#q7yt7）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 部分完成（PR1 测试树模块化已落地；PR2 runner / CI / contract 改动已本地验证，并已补齐 stacked PR 的 CI 触发，待新 head push + PR CI 收口）
- Lifecycle: active
- Catalog note: 双 PR：先测试树模块化，再 runtime/CI 合同收口

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
- 本地 profile wall time（2026-07-09，热缓存）：
  - `lightweight`: 281 tests, `real 3.83s`
  - `stateful_sqlite`: 1040 tests, `real 66.97s`
  - `archive_file_io`: 195 tests, `real 29.14s`
- 本地 top offenders 采样：
  - `lightweight`: `raw_compression_budget_stops_after_first_batch_when_budget_is_exhausted` (`2.229s`)
  - `stateful_sqlite`: `pool_openai_v1_compact_overload_falls_back_to_alternate_route_before_body_forward` (`15.635s`)
  - `archive_file_io`: `send_pool_request_with_failover_returns_owner_unavailable_for_encrypted_session_lock` (`11.828s`)

## Remaining Gaps

- 待补充：PR2 修复 stacked PR CI 触发后的新 head review-loop / PR CI 证据。
- 待补充：merge 后 `CI Main` 对最慢 backend required job `<= 6m30s` 的最终服务端真相。

## Related Changes

- PR1 branch: `th/backend-test-modularization`
- PR2 branch: `th/backend-test-runtime-profiles`

## References

- `./SPEC.md`
- `./HISTORY.md`
