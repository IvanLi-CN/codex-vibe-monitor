# 建立全局 UI 规范文档体系 - Implementation

## Current State

- Canonical spec: `docs/specs/quhzx-ui-guidelines-system/SPEC.md`
- Implementation summary: 已完成（5/5）
- Frontend source organization: 页面级与领域级 React 组件、stories、fixtures 和 model helpers 归入 `web/src/features/<domain>/` 或 `web/src/storybook/`；`web/src/components/` 仅保留 `ui/` 基础组件层。

## Migrated Implementation Notes

## 状态

- Status: 已完成（5/5）
- Created: 2026-03-18
- Last: 2026-03-18

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `bunx dprint check docs/ui docs/specs/quhzx-ui-guidelines-system README.md`
- `cd web && bun run build-storybook`
- 路径存在性检查：文档中引用的实现文件、stories 与规范文件均可解析

## 文档更新（Docs to Update）

- `docs/ui/README.md`
- `docs/ui/foundations.md`
- `docs/ui/components.md`
- `docs/ui/patterns.md`
- `docs/ui/data-viz.md`
- `docs/ui/storybook.md`
- `docs/ui/impeccable-audit.md`
- `docs/specs/README.md`
- `docs/specs/quhzx-ui-guidelines-system/SPEC.md`
- `README.md`

## Migrated Implementation Sections

### Quality checks

- 保持 docs-only 变更
- 文档不添加修订版标记或版本后缀
- PR 标签满足 `type:docs` 与 `channel:stable`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 创建 `docs/ui/` 文档体系与主入口
- [x] M2: 完成 foundations / components / patterns / data-viz / storybook 五份规范正文
- [x] M3: 创建 spec 并同步索引与 README 入口
- [x] M4: 完成本地验证（dprint、路径检查、Storybook build）
- [x] M5: 完成 docs-only fast-track 交付（提交、PR、checks、review-loop、spec-sync）
