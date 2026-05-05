# 修正剩余 `XY_*` 环境变量命名 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/ts4zf-rename-remaining-xy-envs/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-10: 创建 spec，冻结剩余公开 `XY_*` env 的 rename matrix、immediate cutover 策略与 breaking migration 口径。
- 2026-03-10: 完成实现、文档、回归测试、review-loop 与 fast-track 交付；PR [#110](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/110) 已创建，labels=`type:major` + `channel:stable`，checks green。
