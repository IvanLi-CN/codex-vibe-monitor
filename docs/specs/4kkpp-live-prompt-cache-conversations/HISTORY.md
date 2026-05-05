# Live 对话统计（按 Prompt Cache Key）— 无统计表方案 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/4kkpp-live-prompt-cache-conversations/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-03: 新建规格，冻结“无统计表 + 轻缓存”实现策略。
- 2026-03-03: 完成后端聚合接口、表达式索引、5s 轻缓存与前端 Live 对话统计区块，质量门槛（cargo + web test/build）通过并进入 fast-track 交付链路。
- 2026-03-03: review-loop 修复并发细节：后端 singleflight 增加取消安全清理，前端请求参数切换改为读取最新 limit，避免并发刷新使用旧值。
- 2026-03-03: 按验收反馈将移动端（`<sm`）切换为列表卡片模式，并补充 Storybook 桌面/移动截图用于 PR 展示。
