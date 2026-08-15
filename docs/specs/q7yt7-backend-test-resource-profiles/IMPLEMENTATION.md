# 后端测试资源分层模块化与运行时预算 实现状态（#q7yt7）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 已完成全量 PR required-job 成本收敛候选；保持三个独立 backend profiles，并以当前 PR head 完成冷/热两轮三分钟预算验收
- Lifecycle: active
- Catalog note: 三路 backend profile 合同保持不变；本轮收敛 Rust 编译缓存、Archive file fixture、PR smoke build 与非 backend required-job 资源边界

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
- PR run `31825458818` 扩大了预算口径：`Lint & Format Check` 为 `348s`，Lightweight / Stateful / Archive 为 `277s` / `324s` / `382s`，`Build Artifacts` 为 `537s`。候选要求每个 required job 在同一 PR head 的首轮冷 SHA 与第二轮热 SHA 都不超过 `180s`，并使 required runner 总秒数至少下降 `20%`。
- 生产行为不变的测试专用成本收敛已实现：
  - fallback 429 retry delay 只在 `cfg(test)` AppState override 存在时注入；生产构造不含该字段，继续使用原有退避公式。
  - 普通 test state 的 no-available-account wait 为零；验证真实时间预算的测试显式清除 fallback override 或提供自己的非零 wait。
  - replay snapshot 保留正式 `1 MiB` wrapper，并用私有 threshold 参数让大请求语义测试以较小输入覆盖 file-backed 分支；正式阈值边界另有回归测试。
- 当前 Stateful profile 共 `1213` 个用例。完整热运行矩阵每档两次均通过：
  - `4` threads：`134.108s`、`83.952s`
  - `6` threads：`63.593s`、`62.874s`
  - `8` threads：`57.287s`、`67.727s`
  - `8` threads 平均最快，`6` threads 在最快档位 `10%` 内且线程更低，runner 固定为 `6`。
- 当前 top offenders 主要是 SQLite write-lock/backfill、retention/archive 与系统 raw metrics 路径；它们保留真实锁、archive 与 retention 行为。普通 current-schema-only 测试已进一步迁到 template pool，包括服务层级回填、成本回填、内存启动错误分类、定价重载和默认 source-scope 场景。
- 上游账户 stateful suite 的共享 `test_pool()` 在 runner 提供 schema template 时同样复用唯一 shared-memory template pool，消除每个 `AppState` 的重复 `ensure_schema`；Stateful 内有 `82` 个调用点。它在未提供 template 的本地直跑及 Archive/File I/O profile 中严格保留原有 `ensure_schema` 路径。最新本地完整 Stateful receipt 为 `1213/1213`、nextest execution `42.507s`、runner `76s`。
- runner 提供可选 `--archive-file`，本地 archive replay 的三个完整 profile 都能通过。CI archive 仍未进入最终 candidate：分离 producer 的 run `31811122919` 从 workflow start 到 Stateful completed 为 `504s`，虽然 backend runner 总计 `872s`；把 archive 构建移到 Stateful required job 的 run `31813566813` 为 `433s`，三个 backend runner 总计 `750s`。两次均未达到 `<= 390s` 的关键路径门槛，因此 workflow 与 quality-gates 维持三个独立 job；只有同一 PR head 的双重 CI 阈值和稳定 archive profile 证据都成立时，才允许重新提议 workflow 变更。
- runner 在启动 Stateful profile 前由一次真实 `ensure_schema` 生成私有 file template；每个 nextest 子进程用 SQLite backup API 把它复制到唯一 shared-memory SQLite。它通过 schema object/default-data parity、两条 pooled connection 双向写入可见性和跨数据库隔离回归；普通 state 保留原有四连接池。共享 in-memory serialize/deserialize 原型因连接不可见、逐条 SQL dump 因每个子进程重复构建而回退；直接向 shared-memory state 复制文件的原型因 SQLite snapshot lock 而被拒绝。
- 当前候选将 Cargo registry/git 与 target cache 分离。target cache key 同时绑定 lockfile、manifest 和 Rust source fingerprint，clippy 与 nextest 使用不同 target namespace，避免旧 key 让互不兼容的产物长期命中。`profile.test` 关闭 debug information；PR Docker smoke 使用专用 `ci-smoke` profile，生产 release profile 不变。
- Archive/File I/O runner 也会先生成一次真实 current-schema file template。普通 retention/archive DB tests 从它复制到各自唯一文件；三个 legacy migration backfill tests 显式保留 fresh `ensure_schema`，gzip、路径、文件损坏与 write-lock tests 同样不走 template。文件副本的回归覆盖验证模板 mutation 不会泄漏到后续 test DB。
- `CI PR` 和 `CI Main` 将 Rust `Lint & Format Check` 与 `Repository Tooling Checks` 分离，并把原有前端合并 job 拆为 `Front-end Tests`、`Storybook Accessibility Tests`、`Docs & Web Demo Build`；E2E 和三个 backend jobs 保持不变。release snapshot 等待完整的拆分拓扑；quality-gates 声明、contract fixtures 与 live-rules fixtures 同步使用新 names。

## Convergence Contract

- 每个候选 head 都必须重新绑定同一 PR head 的首轮冷 SHA 与第二轮热 SHA CI 性能证据；两轮的每个 required job 都须 `<= 180s`，required runner 总秒数须较 run `31825458818` 至少下降 `20%`。祖先 head 的 receipt 只能作为诊断基线，不能作为合并凭证。
- nextest archive CI 仍是未保留的实验；它不得绕过上述完整预算，也不得在不能同时改善关键路径和总 runner 成本时改变 workflow/quality-gates 契约。

## Related Changes

- `perf(ci): restore required test budget`（当前 candidate）

## References

- `./SPEC.md`
- `./HISTORY.md`
