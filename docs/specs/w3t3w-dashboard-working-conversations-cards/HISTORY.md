# Dashboard：工作中对话卡片替换 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/w3t3w-dashboard-working-conversations-cards/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-04-06: fresh review-proof 补充同一工作锚点并列时的边界约束，前端 SSE merge 与 Dashboard 最终 `20` 卡 cap 都改为直接按 `createdAt DESC` 打破 tie，避免本地超额裁切再次被活动时间带偏。
- 2026-04-06: PR 收敛期根据 fresh review-proof 补充前端 SSE merge 与 Dashboard mapper 修正，5 分钟工作集与最终 `20` 卡 cap 在本地超额裁切时继续按工作锚点保留可见对话，最终展示顺序仍保持 `createdAt DESC`。
- 2026-04-06: PR #295 完成 labels、远端 checks 与 fresh review-proof 收敛，快车道终态更新为 merge-ready。
