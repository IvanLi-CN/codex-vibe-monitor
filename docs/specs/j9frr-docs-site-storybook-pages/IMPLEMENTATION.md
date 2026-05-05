# 文档站与 Storybook GitHub Pages 同构发布 - Implementation

## Current State

- Canonical spec: `docs/specs/j9frr-docs-site-storybook-pages/SPEC.md`
- Implementation summary: 已完成（5/5）

## Migrated Implementation Notes

## 状态

- Status: 已完成（5/5）
- Created: 2026-03-19
- Last: 2026-03-19

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cd docs-site && bun run build`
- `cd web && bun run storybook:build`
- `bash .github/scripts/assemble-pages-site.sh docs-site/doc_build web/storybook-static .tmp/pages-site`
- 浏览器验收：`docs-site` 首页、`storybook.html` 重定向、一个 Storybook docs 深链，以及按 `DOCS_BASE` 子路径提供的 assembled `/storybook/` 访问

## 文档更新（Docs to Update）

- `README.md`: 增加 public docs / docs-site / Storybook / Pages 入口与本地 URL 合同
- `web/README.md`: 替换模板内容，明确 app / Storybook / docs-site 协作方式
- `docs/ui/README.md`: 明确 public docs 与内部 UI 规范的边界
- `docs/ui/storybook.md`: 增加 public docs/storybook 回链说明
