# 后端测试资源分层模块化与运行时预算 演进历史（#q7yt7）

> 这里记录会影响 Agent 理解“为什么一步步变成现在这样”的关键演进；单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 2026-07-09：创建 follow-up spec，冻结“一个 spec + 两个连续 PR”的交付形态。
- 2026-07-09：顶层资源分层固定为 `lightweight`、`stateful_sqlite`、`archive_file_io`，不再接受编号或字母切片作为长期命名。
- 2026-07-09：owner-facing backend required checks 冻结为三个 job：`Backend Tests (Lightweight)`、`Backend Tests (Stateful SQLite)`、`Backend Tests (Archive / File I/O)`。
- 2026-07-09：运行时目标冻结为 `CI Main` 中最慢 backend required job 的 wall time `<= 6m30s`。
- 2026-07-09：PR1 完成两条测试树的真实模块化入口，`pool_failover_window_*`、`tests_part_*` 与 `parts.rs` 退出代码真相源。

## Key Reasons / Replacements

- `4tgau` 已经完成 crate-root / 生产模块边界和浅层测试入口模块化；更深测试切片治理与 runtime budget 需要新的长期主题承接。
- 旧 `pool_failover_window_*` 与 `tests_part_*` 命名无法承载后续 nextest/profile-aware 分组与 owner-facing CI 诊断，因此必须退出长期真相源。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
