# Daily timeseries archive continuity and subday bucket guard - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/nm7ep-daily-timeseries-rollup-continuity/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-11: 初始化 hotfix spec，冻结“daily timeseries after archive must stay continuous / no schema change / no detail backfill”范围。
- 2026-03-11: daily timeseries 已接入 `invocation_rollup_daily`，并补充 archived day、same-day mixed bucket、proxy-only scope 的后端回归测试。
- 2026-03-11: review-loop 发现 rollup 为 Asia/Shanghai 日粒度后，补充“仅在请求时区日边界匹配时合并 rollup”的保护逻辑与对应 UTC / Asia-Singapore 回归测试。
- 2026-03-11: 完成 shared testbox 生产快照验证，确认 Asia/Shanghai 日图恢复 archived rollup，UTC 等不匹配时区不会误并入 rollup。
- 2026-03-19: 扩展热修范围到归档窗口下的 subday bucket guard；`/api/stats/timeseries` 新增 `effectiveBucket` / `availableBuckets` / `bucketLimitedToDaily`，统计页据此自动限制 bucket 并回退 stale 选择。
- 2026-03-19: 本地验证通过，PR #184 已创建，当前收口到 `PR ready`。
