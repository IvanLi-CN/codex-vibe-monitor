# 线上失败请求分类治理与可观测性增强 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/gwpsb-proxy-failure-hardening/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-02-24: 初始化规格并冻结实现范围与验收标准。
- 2026-02-24: 完成失败分类与统计口径改造，PR #51 进入收敛。
- 2026-02-24: 修复历史回填 `is_actionable` 误判与 Label Gate 读取陈旧标签上下文问题。
- 2026-03-11: 修复 Responses SSE `response.failed` / `type:error` 被误记为 `success` 的问题，并补齐上游失败明细与历史回填。
