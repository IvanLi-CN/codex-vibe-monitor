# 启动回填 SQLite 锁冲突修复 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/pr834-startup-backfill-sqlite-lock/SPEC.md`
- Legacy source: `docs/plan/0010:startup-backfill-sqlite-lock/PLAN.md`
- Legacy deletion is intentionally deferred until explicit approval.

## Migrated History Notes

## Change log

- 2026-02-24：完成 M1-M4；实现启动回填批处理与锁冲突重试，显式强制 SQLite `WAL + busy_timeout=30s`，并补齐回归测试（PR #49）。
- 2026-02-24：评审补充修复：启动回填增加 `snapshot_max_id` 上界，仅处理启动前遗留行，避免并发新增记录导致启动回填循环长期不结束。
- 2026-02-24：评审补充修复：锁冲突判定优先使用 SQLx/SQLite 结构化错误码（`5/6`，`SQLITE_BUSY/SQLITE_LOCKED`），文本匹配仅保底兜底。
- 2026-02-24：补充回归测试：新增 `snapshot_max_id` 上界行为测试与结构化 SQLite 错误码判定测试，覆盖防御性修复路径。
- 2026-02-24：补充回归测试：新增“非锁错误不重试”测试，断言启动回填仅在锁冲突时重试，普通错误首轮即失败返回。
