# 后端测试资源分层模块化与运行时预算 实现状态（#q7yt7）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 已完成全量 PR required-job 成本收敛候选；保持三个独立 backend profiles，并以当前 PR head 完成冷/热两轮三分钟预算验收
- Lifecycle: active
- Project-owned validation target: `Dockerfile` `backend-test` (`rust:1.96.0-bookworm`, `cargo-nextest 0.9.138`, checksum-verified amd64 asset) invokes the profile runner with an isolated writable `/tmp` workspace.
- Shared-testbox entrypoint: `compose.backend-test.yml` builds that same target without changing the profile command or test-thread contract.
- Representative-scale acceptance: `src/tests/stateful_sqlite/representative_scale_acceptance.rs` is a deterministic, data-blind exact-oracle test selected by `--test-filter`; it crosses the retired 64 MiB aggregate source boundary and enforces 30-second bootstrap / 1800-second all-time budgets.
- Release consumes the same target SHA's successful CI Main result and runtime-image smoke. Representative-scale acceptance remains a deterministic PR/Main gate and has no external receipt or production-copy input.
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
  - `--partition hash:N/M`（仅用于受校验的 Stateful SQLite 分片）
- PR2 已将不属于 `src/tests/**` / `src/upstream_accounts/tests/**` 的 136 个内联 backend unit tests 并回 `lightweight` profile，避免 profile split 造成 coverage 回归。
- PR2 已把 owner-facing backend required checks 更新为三个 job，并同步 `.github/quality-gates.json`、contract fixtures、release snapshot 自测与 live quality-gates fixtures。
- 当前主线将 Stateful SQLite 完整集合并行分为 `hash:1/2` 与 `hash:2/2`，由单一 `Backend Tests (Stateful SQLite)` 聚合 required check 收口；成功 CI Main 后的 `backend-test` 镜像发布独立为 `workflow_run`，不再阻塞 CI Main 完成。
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
- runner 提供可选 `--archive-file`，本地 archive replay 的三个完整 profile 都能通过。早期 archive producer 的 run `31811122919` 为 `504s`，而把 archive 构建移到 Stateful required job 的 run `31813566813` 为 `433s`，均未达到 Stateful `<= 390s`。三分钟预算候选改为独立的 auxiliary producer：它一次编译当前 head、上传 archive，三个既有 required backend jobs 只下载并回放自己的完整过滤集合。backend jobs 在 producer 失败时仍会启动并因 artifact 缺失而失败，禁止被静默标记为 skipped。该 producer 不在 branch protection 中，只有当前 head 冷/热两轮同时满足 180 秒、390 秒与 runner 成本门槛时才可保留。
- runner 在启动 Stateful profile 前由一次真实 `ensure_schema` 生成私有 file template；每个 nextest 子进程用 SQLite backup API 把它复制到唯一 shared-memory SQLite。它通过 schema object/default-data parity、两条 pooled connection 双向写入可见性和跨数据库隔离回归；普通 state 保留原有四连接池。共享 in-memory serialize/deserialize 原型因连接不可见、逐条 SQL dump 因每个子进程重复构建而回退；直接向 shared-memory state 复制文件的原型因 SQLite snapshot lock 而被拒绝。
- 当前候选将 Cargo registry/git 与 target cache 分离。nextest target key 同时绑定 lockfile、manifest 和 Rust source fingerprint；Lint 只读恢复 registry/git，archive producer 与 PR smoke 才读取分离的 source-key target，避免 Clippy 写入无源码 fingerprint 的 legacy `target`，并使 smoke 的 private test profile 复用 archive producer 的依赖。source-key target 只可在 archive build 成功后保存，失败 run 不得写入后续 cold receipt 可恢复的 cache。test-only profile 固定为 `debug=0`；`codegen-units=256` 会改变 profile fingerprint 并使已恢复依赖全部失效，因此被撤回。PR smoke 的 binary、web bundle 与同版本 Xray archive 由 `PR Smoke Artifact Producer` 组装后上传；`Build Artifacts` 只下载该产物并构建私有 `ci-smoke-runtime`、运行真实容器 smoke。producer 失败时 required build job 以显式结果检查失败，绝不被跳过。其 Ubuntu 24.04 runtime 与 host binary 的 glibc 对齐，生产默认 Bookworm `runtime` target 和 release profile 不变。
- Archive/File I/O runner 也会先生成一次真实 current-schema file template。普通 retention/archive DB tests 从它复制到各自唯一文件；四个 legacy migration/backfill tests（包括 blob-link trigger 回填）显式保留 fresh `ensure_schema`，gzip、路径、文件损坏与 write-lock tests 同样不走 template。会再次执行 `ensure_schema` 的 fresh file fixture 固定单连接，避免一个 SQLite DDL 序列在 pool 内切换连接后重放已存在 trigger；普通 template copy 仍保留多连接可见性覆盖。文件副本的回归覆盖验证模板 mutation 不会泄漏到后续 test DB。
- `CI PR` 和 `CI Main` 将 Rust `Lint & Format Check` 与 `Repository Tooling Checks` 分离，并把原有前端合并 job 拆为 `Front-end Tests`、`Storybook Accessibility Tests`、`Docs & Web Demo Build`；三个 backend jobs 保持不变。Web type-check 从 PR smoke packaging 移到独立 tooling job，避免重复阻塞 Docker smoke。Records Overlay 与 Web Demo E2E 仍运行完整两套 spec、独立 server/report/result 目录并各固定单 worker；`PR E2E Test Producer` 完整运行它们，`Records Overlay E2E` required gate 在 producer 失败时显式失败，因此每个 PR 和 merge-group 都保留完整 E2E 覆盖而不把 producer failure 隐藏为 skipped。quality-gates contract 同时验证 producer 命令中包含两套 spec、独立输出目录和 mock-only Web Demo endpoint，防止 gate 存在但实际覆盖缩减。hosted runner 已提供 Chromium runtime 依赖，因此只下载 Playwright Chromium，不重复 apt 安装字体。release snapshot 等待完整的拆分拓扑；quality-gates 声明、contract fixtures 与 live-rules fixtures 同步使用新 names。

## Convergence Contract

- 每个候选 head 都必须重新绑定同一 PR head 的首轮冷 SHA 与第二轮热 SHA CI 性能证据；两轮的每个 required job 都须 `<= 180s`，required runner 总秒数须较 run `31825458818` 至少下降 `20%`。祖先 head 的 receipt 只能作为诊断基线，不能作为合并凭证。
- nextest archive CI 是受控 auxiliary 实验；它不得改变 required check 名称、绕过完整预算，或在不能同时改善关键路径和总 runner 成本时保留 workflow/quality-gates 契约。

## Related Changes

- `perf(ci): restore required test budget`（当前 candidate）

## References

- `./SPEC.md`
- `./HISTORY.md`
