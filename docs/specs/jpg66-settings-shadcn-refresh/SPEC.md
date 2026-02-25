# 设置页切换为 shadcn 风格并优化亮/暗主题可读性（#jpg66）

## 状态

- Status: 进行中
- Created: 2026-02-25
- Last: 2026-02-25

## 背景 / 问题陈述

- 当前设置页采用 DaisyUI 原子类混合手写样式，局部控件（尤其预置模型启用控件）在视觉密度与内边距上不协调。
- 主人明确要求改为 shadcn 风格，并保持亮色 / 暗色两套主题都清晰可读。

## 目标 / 非目标

### Goals

- 在 `web` 侧引入 shadcn 风格基础组件（至少覆盖 Settings 页所需控件）。
- 重构设置页布局与交互控件，使卡片、表格、开关在亮/暗主题下层级统一、间距自然。
- 保持现有功能行为不变（配置保存、价格表编辑、自动保存逻辑）。

### Non-goals

- 不一次性迁移全站所有页面到 shadcn。
- 不改动后端 API、数据结构与持久化逻辑。

## 范围（Scope）

### In scope

- `web/package.json` 与锁文件：新增 shadcn 风格组件所需依赖。
- `web/src/components/ui/**`：新增基础 UI 组件（Button/Input/Card/Switch 等）。
- `web/src/pages/Settings.tsx`：替换原设置页控件与布局样式。
- 必要的样式/工具文件调整（如 `cn` 工具函数）。

### Out of scope

- `src/` Rust 后端。
- 除 Settings 之外页面的全面视觉重写。

## 验收标准（Acceptance Criteria）

- Given 位于设置页，When 切换亮/暗主题，Then 所有主要区域（标题、卡片、输入、开关、表格）可读性正常且无明显错位。
- Given 预置模型列表，When 启用/停用任一模型，Then 交互反馈清晰且不出现异常内边距/对齐问题。
- Given 价格表编辑任意字段，When blur 或自动保存触发，Then 行为与重构前一致。
- Given 执行 `npm run build`，Then 前端构建通过。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 完成 specs 建档并冻结本轮改造范围。
- [x] M2: 引入 shadcn 风格基础组件与依赖（并移除 DaisyUI 依赖与插件）。
- [x] M3: 完成 Settings 页面重构并通过构建验证（`npm run build`）。
- [x] M4: 产出亮/暗主题可复验截图，供主人确认视觉结果。

## 进度备注

- 已完成 DaisyUI 相关库与插件移除，源码内不再使用 DaisyUI 组件类。
- Settings 页重点修复了被反馈的两处区域：
  - 代理配置中的开关行（文本区与开关垂直对齐、内边距统一）。
  - 价格表表头/单元格/输入框与来源徽标、删除按钮的内边距与对齐。
- Playwright 在当前环境下快照为空白；已使用 Chrome DevTools 实机验收并产出截图。
