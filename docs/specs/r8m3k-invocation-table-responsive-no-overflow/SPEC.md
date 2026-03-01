# InvocationTable 响应式修复：lg+ 无横向滚动、sm 及以下列表化（#r8m3k）

## 状态

- Status: 已完成（5/5）
- Created: 2026-03-02
- Last: 2026-03-02

## 背景 / 问题陈述

- 当前请求记录表在部分桌面视口会出现不合理横向滚动，导致首屏信息可见性下降。
- 移动端沿用表格结构可读性较差，且容易引入页面级横向溢出。

## 目标 / 非目标

### Goals

- 在 `lg` 及以上视口彻底消除 InvocationTable 横向滚动条。
- 在 `sm` 及以下（本次实现为 `<768px`）切换为列表卡片视图，保留同等信息密度与详情展开能力。
- 为 Dashboard 与 Live 两页补齐跨视口 E2E 回归。

### Non-goals

- 不修改后端 API / SSE / SQLite。
- 不改造 InvocationTable 之外的其它业务表格组件。

## 范围（Scope）

### In scope

- `web/src/components/InvocationTable.tsx`
- `web/tests/e2e/invocation-table-layout.spec.ts`
- `docs/specs/README.md`

### Out of scope

- `src/**` Rust 后端
- 非 InvocationTable 的页面布局

## 需求（Requirements）

### MUST

- `>=1024px`：`scrollWidth - clientWidth <= 1`，不可出现横向滚动。
- `<768px`：仅显示列表视图；页面级 `documentElement.scrollWidth - clientWidth <= 1`。
- Dashboard / Live 两页均满足上述约束。
- 列表与表格均支持展开详情，且详情字段一致。

### SHOULD

- 仅改样式与前端展示，不引入 API/type 破坏性变更。
- 现有加载态、空态、错误态行为不回退。

## 验收标准（Acceptance Criteria）

- Given `375 / 768 / 1024 / 1280 / 1440 / 1873` 视口、Dashboard 与 Live 两页，When 渲染 InvocationTable，Then 布局与溢出断言全部通过。
- Given 移动端列表首项，When 点击展开按钮，Then 详情可正常展开并可读。
- Given 桌面端表格首项，When 点击展开按钮，Then 详情可正常展开并且按钮默认可见。

## 质量门槛（Quality Gates）

- `cd web && npm run build`
- `cd web && npm run test`
- `cd web && npm run test:e2e -- tests/e2e/invocation-table-layout.spec.ts`

## 里程碑（Milestones）

- [x] M1: 新建规格并锁定断点契约与验收矩阵。
- [x] M2: 完成 InvocationTable 响应式双视图改造与桌面防溢出。
- [x] M3: 扩展 E2E 回归并通过本地验证。
- [x] M4: 快车道收敛（push + PR + checks + review-loop）。
- [x] M5: 回写规格状态与变更记录。

## 风险 / 假设

- 假设：`md (768~1023)` 采用表格视图。
- 风险：超长无空格文本可能触发单元格挤压，需通过截断策略兜底。

## 变更记录（Change log）

- 2026-03-02: 创建规格，进入实现阶段。
- 2026-03-02: `InvocationTable` 完成双视图改造（`<768` 列表卡片、`>=768` 表格），并统一展开详情行为。
- 2026-03-02: 表格视图收敛列宽预算与长文本截断策略，覆盖长 endpoint / 长错误串场景，`lg+` 无横向滚动。
- 2026-03-02: E2E 扩展为 Dashboard/Live × `375/768/1024/1280/1440/1873` 全矩阵，校验视图形态、溢出约束与详情交互。
- 2026-03-02: 本地验证通过：`npm run test`、`npm run build`、`E2E_BASE_URL=http://127.0.0.1:4173 npm run test:e2e -- tests/e2e/invocation-table-layout.spec.ts`。
- 2026-03-02: 快车道交付 PR [#79](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/79)，标签 `type:patch` + `channel:stable`。
