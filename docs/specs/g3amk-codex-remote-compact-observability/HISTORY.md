# Codex 远程压缩请求记录、展示与计费接入 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/g3amk-codex-remote-compact-observability/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-09: 创建规格，冻结 compact 识别、统计口径、计费口径与“不注入 `service_tier`”边界。
- 2026-03-09: 已完成后端 compact capture / pricing / stats 接入，以及 InvocationTable compact 标记与 settings 文案改动。
- 2026-03-09: 已完成本地 Rust / web 验证与 review-loop 审查；远端 PR、checks 与 merge readiness 已收敛。
- 2026-04-27: 补充 future-only compact prompt-cache 归因要求；缺 key 的新 compact 可通过同一客户端稳定指纹继承最近普通 responses 的 `promptCacheKey` / `stickyKey`，旧记录不 backfill。
