# 号池流式上游误用整请求超时 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/ynr8z-pool-stream-total-timeout/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-17: 创建 spec，冻结根因、边界与回归标准。
- 2026-03-17: 完成 `HttpClients` 分工修复；号池 live upstream 改走无整请求总超时 client，并为“首 chunk 超时 / OAuth 非流式读体超时”补回显式预算，`cargo test` 全量 395 项通过。
