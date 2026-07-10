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

## Key Reasons / Replacements

- `4tgau` 已经完成 crate-root / 生产模块边界和浅层测试入口模块化；更深测试切片治理与 runtime budget 需要新的长期主题承接。
- 旧 `pool_failover_window_*` 与 `tests_part_*` 命名无法承载后续 nextest/profile-aware 分组与 owner-facing CI 诊断，因此必须退出长期真相源。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
