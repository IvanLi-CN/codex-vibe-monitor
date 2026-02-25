# 修复 InvocationTable 异常横向滚动（#26knq）

## 状态

- Status: 已完成
- Created: 2026-02-26
- Last: 2026-02-26

## 背景 / 问题陈述

- 线上 `https://vibe-code.ivanli.cc/#/dashboard` 的“最近 20 条实况”表格在桌面宽度下出现了不合理横向滚动。
- 该滚动并非由视口不足导致，而是 `InvocationTable` 固定最小宽度与外层容器宽度/内边距叠加造成。
- 直接影响：最后一列展开箭头默认处于可视区域之外，首屏交互可达性下降。

## 目标 / 非目标

### Goals

- 修复 `InvocationTable` 在常见桌面宽度下的伪横向溢出。
- 保证展开箭头在 `scrollLeft=0` 时可见。
- 新增 E2E 回归，覆盖 Dashboard 与 Live 页面共享表格场景。

### Non-goals

- 不调整后端接口、数据结构与 SSE 行为。
- 不改造其他页面的非 InvocationTable 表格布局。

## 范围（Scope）

### In scope

- `web/src/components/InvocationTable.tsx`
- `web/tests/e2e/invocation-table-layout.spec.ts`（新增）
- `docs/specs/README.md` 与本规格文档的状态同步

### Out of scope

- `src/**` Rust 后端
- Settings 页面与其它无关组件

## 需求（Requirements）

### MUST

- 去除桌面宽度下的“非必要横向滚动”（滚动差值应接近 0）。
- 首行展开按钮默认可见（不被容器右侧裁剪）。
- Dashboard 与 Live 页面同时生效（共享组件修复）。
- 新增 E2E 用例，验证上述行为。

### SHOULD

- 不引入 API/type 层变更。
- 保持原有字段截断、展开详情、状态显示行为不回退。

### COULD

- 如测试稳定性需要，可补充轻量 `data-testid`。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 用户在 Dashboard / Live 查看 InvocationTable 时，默认无需横向滚动即可看到展开按钮。
- 点击展开按钮后，详情行行为与现状一致。

### Edge cases / errors

- 在较窄视口（例如手机）允许出现合理横向滚动，不作为本次缺陷。
- 当列内容超长时继续使用现有截断策略，不强制换行导致布局抖动。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name）                        | 类型（Kind） | 范围（Scope） | 变更（Change）  | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes）            |
| ----------------------------------- | ------------ | ------------- | --------------- | ------------------------ | --------------- | ------------------- | ------------------------ |
| InvocationTable props / API payload | internal     | internal      | Modify(UI only) | None                     | web             | Dashboard, Live     | 仅样式与测试可观测性调整 |

### 契约文档（按 Kind 拆分）

- None

## 验收标准（Acceptance Criteria）

- Given Dashboard 或 Live 页面在 `1280x900` / `1440x900` / `1873x900`，When 渲染 InvocationTable，Then 滚动容器 `scrollWidth - clientWidth <= 2`。
- Given 表格首行，When `scrollLeft=0`，Then 展开按钮右边界不超过容器右边界。
- Given 用户展开/收起详情，When 进行交互，Then 功能行为与修复前一致。
- Given 执行 E2E 与前端构建，When 测试运行，Then 新增用例通过且构建通过。

## 实现前置条件（Definition of Ready / Preconditions）

- 范围、验收标准、流程类型已冻结（快车道）。
- 无后端接口变更需求。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- E2E tests: 新增 `invocation-table-layout.spec.ts` 并通过。

### UI / Storybook (if applicable)

- Stories to add/update: None

### Quality checks

- `cd web && npm run build`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增规格索引并同步状态
- `docs/specs/26knq-invocation-table-overflow/SPEC.md`: 同步里程碑与变更记录

## 计划资产（Plan assets）

- Directory: `docs/specs/26knq-invocation-table-overflow/assets/`
- In-plan references: None

## 资产晋升（Asset promotion）

- None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: InvocationTable 去除固定最小宽度导致的伪横向溢出，并保持现有展示行为。
- [x] M2: 新增 Dashboard + Live 共享表格布局 E2E 回归用例。
- [x] M3: 完成 `npm run build` 与 E2E 验证。
- [x] M4: 完成快车道交付（commit/push/PR/checks/review-loop）。

## 方案概述（Approach, high-level）

- 将表格宽度策略从固定最小宽度改为容器自适应，优先保证桌面宽度下无伪溢出。
- 通过 E2E 在关键桌面宽度断言滚动差值与展开按钮可见性，防止回归。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：窄屏下列内容可能继续触发合理滚动（预期行为）。
- 需要决策的问题：None。
- 假设（需主人确认）：本次仅修复共享 InvocationTable，不额外调整其它表格组件。

## 变更记录（Change log）

- 2026-02-26: 创建规格并记录线上复现基线（Dashboard: `clientWidth=1108`、`scrollWidth=1152`、`maxScrollLeft=44`；首行展开按钮默认被裁剪约 `31px`）。
- 2026-02-26: 完成组件宽度修复与 E2E 回归（新增 `web/tests/e2e/invocation-table-layout.spec.ts`），本地验证 `npm run build` 与新增 E2E 均通过。
- 2026-02-26: 完成快车道收敛：PR #56 已创建并打上 `type:patch` + `channel:stable`，CI/checks 通过；review-loop 第 1 轮发现的 HashRouter 路径问题已修复并回归通过。

## 参考（References）

- 线上复现地址：`https://vibe-code.ivanli.cc/#/dashboard`
- 相关组件：`web/src/components/InvocationTable.tsx`
