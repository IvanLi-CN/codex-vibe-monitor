# 后端测试资源分层模块化与运行时预算 演进历史（#q7yt7）

> 这里记录会影响 Agent 理解“为什么一步步变成现在这样”的关键演进；单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 2026-07-09：创建 follow-up spec，冻结“一个 spec + 两个连续 PR”的交付形态。
- 2026-07-09：顶层资源分层固定为 `lightweight`、`stateful_sqlite`、`archive_file_io`，不再接受编号或字母切片作为长期命名。
- 2026-07-09：owner-facing backend required checks 冻结为三个 job：`Backend Tests (Lightweight)`、`Backend Tests (Stateful SQLite)`、`Backend Tests (Archive / File I/O)`。
- 2026-07-09：运行时目标冻结为 `CI Main` 中最慢 backend required job 的 wall time `<= 6m30s`。
- 2026-07-09：PR1 完成两条测试树的真实模块化入口，`pool_failover_window_*`、`tests_part_*` 与 `parts.rs` 退出代码真相源。
- 2026-07-09：PR2 将 backend runner 固定为 profile-aware nextest 入口，并把 quality-gates / CI / release snapshot 合同一起切到三路 backend checks。
- 2026-07-09：review-loop 指出 profile split 漏掉生产模块里的内联 backend unit tests；修复后将这 136 个用例并回 `lightweight` profile，避免 required checks coverage 回归。
- 2026-07-09：实际打开 stacked PR 后发现 `CI PR` 只对 `base=main` 触发，无法为 PR2 提供服务端 CI 证据；因此放开 `CI PR` 的 `pull_request` base 过滤，同时保留 `Label Gate` / `Review Policy` 与 live rules 对齐检查继续只绑定 `main`。
- 2026-07-09：修复后的本地热缓存测量显示三个 profile wall time 分别约为 `3.83s`、`66.97s`、`29.14s`，拆分后 critical path 远低于 `6m30s` 预算。
- 2026-07-10：PR #576 合并后的 CI Main 实测 Stateful SQLite job 为 `6m45s`，比预算高 `15s`；完整 1048 个 stateful 用例在本地 4、6、8 threads 下均通过，采用保守的 6-thread runner 上限收敛预算。
- 2026-07-10：PR #579 的 CI Main run `29074132864` 实测三路 backend job 为 `3m10s`、`6m00s`、`4m50s`；Stateful SQLite 最慢 job 比 `6m30s` 预算低 `30s`，runtime budget 完成收口。
- 后续 CI Main run `31706131099` 显示 Stateful SQLite 回归到 `617s`（compile `143s`、test execution `404s`），因此预算口径改为 PR workflow start 至 Stateful job completed `<= 390s`，并要求同一 head 连续两次验证。
- 测试时间成本收敛采用 private/`cfg(test)` 注入：生产 retry/backoff 和 replay threshold wrapper 不变，测试 harness 才能使用零等待或较小 replay threshold；验证正式时间行为的测试必须显式恢复正式 delay。
- Stateful 并发选择不再凭单次最快结果决定；完整 profile 的 4/6/8 threads 各运行两次，选择最快档位 10% 内的最低档位。当前 `1213` 用例矩阵为 4 threads `134.108s` / `83.952s`、6 threads `63.593s` / `62.874s`、8 threads `57.287s` / `67.727s`；8 平均最快但 6 在线 10% 内，因此选择 6 threads。
- current-schema fixture 先拒绝了 shared in-memory serialize/deserialize 原型（第二 pooled connection 不可见）、直接文件副本（SQLite snapshot lock）和逐条 SQL dump（每个 nextest 子进程重复构建，CI 执行回退）。最终由 runner 从一次真实 `ensure_schema` 生成私有 file template，并由每个子进程通过 SQLite backup API 复制到唯一 shared-memory SQLite；schema/default-data parity、pooled 双向写入可见性和跨数据库隔离均有回归覆盖。
- nextest archive 被限定为受控 CI 实验；只有两次相同 PR head 同时满足 Stateful `<= 390s` 和 backend runner 总秒数 `<= 1005s` 才允许改变 workflow，不能只因本地 archive runner 通过而保留。
- archive 的两种 CI 拓扑都已按相同门槛拒绝：run `31811122919` 以独立 producer 分发 archive，Stateful critical path 为 `504s`；run `31813566813` 在 Stateful required job 中构建并分发 archive，critical path 为 `433s`。两次虽然 backend runner 总秒数分别为 `872s` 与 `750s`，仍因未达到 `390s` 关键路径预算而撤回 workflow 变更。
- 在最终独立-job 候选中，进一步把只依赖 current schema 的服务层级回填、成本回填、内存启动错误分类、定价重载和默认 source-scope 测试迁入 schema template pool；legacy migration、文件路径、gzip 与 write-lock 测试继续走真实 `ensure_schema` 和 file fixture。
- 上游账户 Stateful tests 曾经通过共享 `test_pool()` 为每个 `AppState` 调用 `ensure_schema`，使远端执行被重复 DDL 主导。该 helper 现在只在 runner 已提供 template 时调用 current-schema template pool；无 template 的直跑和 Archive/File I/O profile 继续走原始 schema 初始化。该收口把本地完整 Stateful execution 降至 `42.507s`，并保留 `1213` 个用例。
- PR run `31825458818` 证明后端关键路径恢复不足以保证全量反馈：Lint、三个 backend jobs 与 Build Artifacts 仍分别达到 `348s`、`277s` / `324s` / `382s` 与 `537s`。因此预算改为全部 required job 的 cold/hot 两轮 `<= 180s`，并要求 required runner 总秒数至少下降 `20%`。
- Rust cache 从 lockfile-only 的 registry/target 混合 key 改为 registry/git 与 source-fingerprinted nextest target keys 分离。迁移实验表明 target-only restore 不能解包旧三路径 cache，故固定先以原三路径只读恢复 ancestor，再写入新 source-key；clippy 不再写入自己的 target cache。PR smoke 并行生成当前 debug binary 与 web bundle，只封装私有 runtime target，生产默认 Docker target 与 release profile 保持不变。
- Archive ordinary current-schema tests 采用唯一 file template copy；migration/backfill、gzip、路径、文件锁和文件损坏保持真实 fresh schema/file fixture。前端 test coverage 不变，Rust lint 与 repository tooling 也分离；Vitest、Storybook accessibility 和 docs/demo static build 拆为独立 required jobs，release snapshot 与 branch-protection contract 同步等待完整拓扑。
- 冷 SHA 即使恢复 ancestor target，仍必须重新链接当前测试二进制；因此引入不进入 branch protection 的 archive producer，让三个固定 backend required checks 并行回放同一 current-head archive。该拓扑只有在同一 PR head 两次同时满足 180 秒 required job、390 秒 Stateful critical path 和 runner 成本门槛时保留。
- host-built PR smoke binary 不能在 Bookworm glibc runtime 运行，私有 smoke target 改为 Ubuntu 24.04；生产 Bookworm runtime 保持默认。hosted runner 已具备 Playwright Chromium runtime 依赖，E2E 仅下载对应浏览器而不重复 apt 安装字体。两个并发的 Playwright 2-worker process 会使 Chromium 协议超时并触发 retry，因此 E2E 保留完整测试，records 保持单 worker，Web Demo 独占两个 workers。
- archive producer 的 `codegen-units=256` 使 Cargo test profile 与已恢复的 `debug=0` target cache 不兼容，导致依赖重编译。该参数已撤回；维持关闭 debug info，以便与同一 Rust source fingerprint 的 ancestor artifact 复用依赖。
- blob-link legacy backfill 测试曾错误继承 current-schema template；它现在显式创建 fresh schema，确保 `ensure_schema` 真实重放 trigger migration，而普通 Archive fixture 继续使用唯一文件模板副本。
- archive producer 同时恢复 legacy workspace 与 source-key target cache 会重复解包 `target` 并把 Stateful critical path 推至 400 秒。producer 现只读分离 cache；Lint 仍保留 legacy read-only compatibility seed。

## Key Reasons / Replacements

- `4tgau` 已经完成 crate-root / 生产模块边界和浅层测试入口模块化；更深测试切片治理与 runtime budget 需要新的长期主题承接。
- 旧 `pool_failover_window_*` 与 `tests_part_*` 命名无法承载后续 nextest/profile-aware 分组与 owner-facing CI 诊断，因此必须退出长期真相源。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
