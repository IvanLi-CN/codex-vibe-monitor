# 反向代理上游 429 自动重试（设置可配） - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/uwke5-proxy-upstream-429-retry/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-10: 创建 spec，冻结 429 自动重试的范围、设置接口与 exhaustion 语义。
- 2026-03-10: 实现落地：新增 `upstream429MaxRetries` 设置、429 重试 helper、全链路接线与回归测试。
