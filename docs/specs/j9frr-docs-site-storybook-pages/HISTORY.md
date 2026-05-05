# 文档站与 Storybook GitHub Pages 同构发布 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/j9frr-docs-site-storybook-pages/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-19: 创建 spec，冻结 docs-site、Storybook、Pages 与 CI smoke 的首版交付范围。
- 2026-03-19: 完成 docs-site / Storybook / Pages 装配实现、targeted validation 与浏览器验收，进入 fast-track PR 收敛阶段。
- 2026-03-19: 参考 `tavily-hikari` 的 task-based IA，把 public docs 重构为“项目介绍 + 快速开始 + 配置与运行 + 自部署 + 排障 + 开发 + Storybook”分工，并强化自部署读者的最短路径。
- 2026-03-19: 删除独立 `storybook-guide` 页面，改为只保留 `storybook.html` 作为 public docs 的 Storybook 入口。
