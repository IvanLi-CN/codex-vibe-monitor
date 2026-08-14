# 后端测试资源分层模块化与运行时预算 实现状态（#q7yt7）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 已完成测试内部成本收敛，正在以当前 PR head 完成 CI 关键路径验收
- Lifecycle: active
- Catalog note: 三路 profile/required-check 合同保持不变；本轮只收敛 Stateful 执行成本与可逆 archive 入口

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
- `CI Main` run `31706131099` 是本轮基线：Stateful SQLite wall time `617s`，其中 compile `143s`、test execution `404s`；三个 backend jobs 合计 `1257s`。
- 生产行为不变的测试专用成本收敛已实现：
  - fallback 429 retry delay 只在 `cfg(test)` AppState override 存在时注入；生产构造不含该字段，继续使用原有退避公式。
  - 普通 test state 的 no-available-account wait 为零；验证真实时间预算的测试显式清除 fallback override 或提供自己的非零 wait。
  - replay snapshot 保留正式 `1 MiB` wrapper，并用私有 threshold 参数让大请求语义测试以较小输入覆盖 file-backed 分支；正式阈值边界另有回归测试。
- 当前 Stateful profile 共 `1212` 个用例。完整热运行矩阵每档两次均通过：
  - `4` threads：`253.040s`、`176.027s`
  - `6` threads：`170.506s`、`149.079s`
  - `8` threads：`232.304s`、`304.421s`
  - `6` threads 是最快平均档位，因此也是最快档位 `10%` 内的最低线程数，runner 固定为 `6`。
- 当前 top offenders 主要是 SQLite write-lock/backfill、retention/archive 与系统 raw metrics 路径，最长单项约 `9.286s`；它们保留真实锁、archive 与 retention 行为。
- runner 提供可选 `--archive-file`。本地 archive 验证的三个 profile 均通过（lightweight `23s`、Stateful SQLite `169s`、archive/file I/O `89s`，合计 `281s`）。它尚未改变 CI workflow：冷编译 archive 的 dependency 会延长 Stateful critical path，只有 PR CI 的双重阈值证据才允许保留 workflow 变更。
- 曾验证 current-schema 模板 fixture：共享 in-memory 模板在全套并发中发生连接竞争，文件模板副本出现 SQLite snapshot lock。两种方案都未同时满足 pooled visibility、并发写安全和完整 profile 运行时要求，已回退到每个 State fixture 的真实 `ensure_schema`。

## Remaining Gaps

- 当前候选尚缺同一 PR head 的连续两次 CI 性能证据。两次从 workflow start 到 Stateful SQLite completed 均须 `<= 390s`。
- archive CI 仍是未保留的实验：若它不能同时使 backend-related runner seconds `<= 1005s`，不得同步进 workflow/quality-gates 契约。

## Related Changes

- `perf(test): reduce stateful backend runtime`（当前 candidate）

## References

- `./SPEC.md`
- `./HISTORY.md`
