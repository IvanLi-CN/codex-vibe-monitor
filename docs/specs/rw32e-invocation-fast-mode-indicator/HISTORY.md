# 请求列表 Fast 模式标识（service tier 版） - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/rw32e-invocation-fast-mode-indicator/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-07: 创建规格，冻结“仅实际 `service_tier=priority` 算 Fast”口径，并要求以 payload-only + 启动回填实现。
- 2026-03-07: 已完成后端 service tier 采集 / 回填、InvocationTable 图标与详情展示，以及 `cargo test`、`cargo check`、`cd web && npm run test`、`cd web && npm run build`、`cd web && npm run test:e2e -- invocation-table-layout.spec.ts` 验证。
- 2026-03-07: 已创建 PR #93，review-loop 发现并修复了 legacy `serviceTier=null` 时未回退 `service_tier` 的投影问题；合并 `main` 后重新推送，PR 已恢复 `mergeable_state=clean` 且 checks 全部通过。
