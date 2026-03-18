# 建立全局 UI 规范文档体系（#quhzx）

## 状态

- Status: 已完成（5/5）
- Created: 2026-03-18
- Last: 2026-03-18

## 背景 / 问题陈述

- 当前仓库已经有稳定的 UI 实现、Storybook stories 与多份 feature spec，但缺少统一的全局 UI 文档入口。
- 主题 token、基础组件、图表颜色、页面模式与 Storybook 证据口径分散在 `web/src/**`、`web/.storybook/**` 与 `docs/specs/**` 中，后续新增页面时很难快速判断“应该沿用什么”。
- 需要以 docs-only 方式补齐一组长期可引用的规范文档，同时不改动运行时代码与现有视觉实现。

## 目标 / 非目标

### Goals

- 在 `docs/ui/` 下建立多文档 UI 规范体系，而不是单一长文。
- 用“当前真相源 + 后续新增规则 + 已知例外/待治理”的结构，既描述现状，也定义后续新增约束。
- 为后续 feature spec、Storybook 证据与页面实现提供统一入口。
- 按 docs-only fast-track 流程完成验证、提交、PR 与 review 收敛。

### Non-goals

- 不修改 `web/src/**`、`src/**` 或 Storybook 配置的运行时行为。
- 不补新的 UI 实现、组件重构或视觉微调。
- 不把所有历史 feature spec 改写成 design system 文档。

## 范围（Scope）

### In scope

- 新建 `docs/ui/README.md`
- 新建 `docs/ui/foundations.md`
- 新建 `docs/ui/components.md`
- 新建 `docs/ui/patterns.md`
- 新建 `docs/ui/data-viz.md`
- 新建 `docs/ui/storybook.md`
- 更新 `docs/specs/README.md`
- 更新 `README.md` 增加 UI 文档入口

### Out of scope

- `web/src/**`、`src/**`、`web/.storybook/**` 的实现性改动
- 新截图或 PR 视觉资产补采
- 新增 Storybook stories

## 需求（Requirements）

### MUST

- `docs/ui/` 至少包含 6 份文档：`README.md`、`foundations.md`、`components.md`、`patterns.md`、`data-viz.md`、`storybook.md`
- 每份文档都显式引用真实真相源文件，而不是只写抽象原则
- `foundations.md` 必须覆盖 light/dark theme、语义色、surface/shadow、字体/数字、圆角与 spacing 约束
- `components.md` 与 `patterns.md` 必须说明 loading / empty / error / disabled / active / selected 等状态语义
- `data-viz.md` 必须覆盖 count/cost/token 三类指标色与热力图梯度约定
- `storybook.md` 必须记录主题切换、默认 viewport、证据采集口径与关键 story
- 变更保持 docs-only；禁止顺手修改运行时代码

### SHOULD

- `README.md` 为 UI 文档提供稳定入口
- 文档结构与现有 repo 规范一致，便于后续 feature spec 回链
- 使用 bun/dprint 与 Storybook build 对新增文档进行验证

## 验收标准（Acceptance Criteria）

- Given 仓库已存在 UI 实现与 stories，When 打开 `docs/ui/README.md`，Then 可以快速定位 foundations、components、patterns、data-viz、storybook 五个子主题文档。
- Given 阅读任一子文档，When 查阅“当前真相源”，Then 能找到至少一个明确的实现文件或 story 路径。
- Given 阅读 `foundations.md`，When 查看主题与视觉基础规则，Then 能看到 light/dark、语义色、surface、字体、数字、圆角与动效边界。
- Given 阅读 `components.md` 与 `patterns.md`，When 查找状态语义，Then 能明确 loading / empty / error / disabled / active / selected 的统一规则。
- Given 阅读 `data-viz.md`，When 查找图表与数字展示规则，Then 能明确 count/cost/token 的颜色语义与热力图梯度来源。
- Given 执行文档校验，When 运行 `bunx dprint check` 与 `cd web && bun run build-storybook`，Then 命令通过。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `bunx dprint check docs/ui docs/specs/quhzx-ui-guidelines-system README.md`
- `cd web && bun run build-storybook`
- 路径存在性检查：文档中引用的实现文件、stories 与规范文件均可解析

### Quality checks

- 保持 docs-only 变更
- 文档不添加修订版标记或版本后缀
- PR 标签满足 `type:docs` 与 `channel:stable`

## 文档更新（Docs to Update）

- `docs/ui/README.md`
- `docs/ui/foundations.md`
- `docs/ui/components.md`
- `docs/ui/patterns.md`
- `docs/ui/data-viz.md`
- `docs/ui/storybook.md`
- `docs/specs/README.md`
- `docs/specs/quhzx-ui-guidelines-system/SPEC.md`
- `README.md`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 创建 `docs/ui/` 文档体系与主入口
- [x] M2: 完成 foundations / components / patterns / data-viz / storybook 五份规范正文
- [x] M3: 创建 spec 并同步索引与 README 入口
- [x] M4: 完成本地验证（dprint、路径检查、Storybook build）
- [x] M5: 完成 docs-only fast-track 交付（提交、PR、checks、review-loop、spec-sync）

## 方案概述（Approach, high-level）

- 从现有 CSS、theme context、chart token、Storybook preview 与关键 stories 提取稳定事实。
- 用多文档方式拆分基础层、组件层、页面模式层、数据展示层与 Storybook 运维层，避免一份文档过长。
- 保持“实现优先，文档跟随”的真相源顺序，文档负责聚合与约束，不抢跑实现。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：部分规范事实仍依赖页面级 story，而非完全独立的 design token 层。
- 风险：Storybook build 若暴露现有无关问题，需明确区分是文档问题还是仓库既有问题。
- 开放问题：无。
- 假设：当前 bun 与 Storybook 依赖可直接用于 docs-only 验证。

## 变更记录（Change log）

- 2026-03-18: 创建 spec，冻结 docs-only UI 规范补档范围、验收标准与 fast-track 交付路径。
- 2026-03-18: 完成 `docs/ui/` 六份文档、README 入口与本地验证；进入 PR 交付与 review 收敛阶段。
- 2026-03-18: 修复 review 指出的 specs 索引表渲染问题与 foundations spacing 约束缺口，随后同步 `origin/main`、更新 PR #173 到 mergeable clean，并确认 checks green / review-loop clear。
