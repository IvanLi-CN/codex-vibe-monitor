# 统一设置页：代理配置 + 价格配置（替换旧方案） - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/enqq6-settings-pricing-unified/SPEC.md`
- Legacy source: `docs/plan/enqq6-settings-pricing-unified/PLAN.md`
- Legacy deletion is intentionally deferred until explicit approval.

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录 / Change log

- 2026-02-23: 初始化计划并冻结范围与验收标准。
- 2026-02-23: 完成旧方案替换（`/api/settings/proxy-models` 下线、`/settings` 上线、价目表改为 SQLite 持久化并可在线编辑）。
- 2026-02-23: 创建 PR #47，完成本地验证（cargo test / web lint+build / settings e2e）并跟踪 CI Pipeline #142 通过。
