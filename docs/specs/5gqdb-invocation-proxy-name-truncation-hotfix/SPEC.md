# InvocationTable 桌面代理名省略回归热修（#5gqdb）

## 状态

- Status: 部分完成（3/4）
- Created: 2026-03-09
- Last: 2026-03-09

## 背景 / 问题陈述

- `InvocationTable` 的桌面表格代理列原本依赖受限宽度的 badge + `truncate` 组合来兜底超长代理名，避免长无空格文本挤压后续列。
- 2026-03-09 的 compact 观测接入把桌面代理 badge 包进了额外的 `flex-wrap` 容器，破坏了 badge 的宽度约束，导致长 VLESS 名直接覆盖 model 列并污染表格布局。
- 这次回归只影响前端展示；`/api/invocations`、SSE records、`proxyDisplayName` 原值与 compact endpoint 识别链路都保持正常，不需要后端裁剪或 schema 变更。
- 若不及时热修，Dashboard 与 Live 的最近请求表在真实长代理名场景下会继续出现首屏错位，且会削弱 `g3amk` 与 `r8m3k` 已完成的布局承诺。

## 目标 / 非目标

### Goals

- 恢复桌面 `InvocationTable` 代理列的宽度约束，让超长代理名重新在列内省略显示。
- 保留完整 `proxyDisplayName` 数据，只通过 `title`/tooltip 暴露完整文本，不在 UI 侧做截断写回。
- 保持 compact 标记继续只在 endpoint 路径显示，不把 compact 标记重新塞回代理列。
- 为长代理名场景补齐稳定的前端回归断言，防止再次引入桌面覆盖或横向溢出。

### Non-goals

- 不修改 Rust 后端、SQLite、`/api/invocations`、SSE records、pricing 或 compact 采集契约。
- 不改造移动端卡片布局、详情面板信息结构、Fast indicator 或 reasoning badge 语义。
- 不引入新的 API 字段，也不在前端做名称缩写/省略号字符串生成。

## 范围（Scope）

### In scope

- `web/src/components/InvocationTable.tsx` 的桌面代理列布局与稳定测试选择器。
- `web/src/components/InvocationTable.test.tsx`、`web/src/components/InvocationTable.stories.tsx`、`web/tests/e2e/invocation-table-layout.spec.ts` 的长代理名回归覆盖。
- `docs/specs/README.md` 与本 hotfix spec 的状态同步。

### Out of scope

- `src/**` Rust 后端与代理采集逻辑。
- 其他业务表格、Live 代理统计表或 Settings 页。
- compact 端点 pricing 说明文案与既有已完成 spec 的正文重写。

## 需求（Requirements）

### MUST

- 在 `>=1280px` 的桌面表格中，超长代理名必须只在代理列内省略，不能覆盖 model 列或把展开按钮挤出容器。
- `proxyDisplayName` 的完整原值必须仍存在于 DOM title 中，便于鼠标悬浮查看。
- compact 记录必须继续只通过 endpoint 路径样式标识，不恢复代理列 compact badge。
- Dashboard 与 Live 两页都必须覆盖长代理名回归验证。
- 长代理名回归不能重新引入整表横向滚动；`overflowDelta` 继续满足既有 <= 1 约束。

### SHOULD

- 回归测试使用稳定 `data-testid`，避免依赖脆弱的 class 或文案定位。
- Story 与 E2E 使用同一条真实 VLESS 风格长代理名样例，减少场景漂移。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 桌面表格代理列继续以状态 badge 承载代理名称，但 badge 自身必须受单元格宽度约束；内部文本通过 `truncate` 在列宽内省略。
- 移动端列表继续使用现有“状态 badge + 代理名称文本”布局，不新增 compact badge，也不改变 endpoint 区块展示逻辑。
- E2E 通过第二行长代理名记录校验 badge 边界与名称滚动宽度，确保真实回归场景可重现。

### Edge cases / errors

- 长代理名可为无空格 VLESS/VMess 风格字符串；实现不能依赖单词断行。
- 若记录为普通短代理名或 `Direct`，展示与现状一致，不额外添加视觉噪音。
- 若 endpoint 为 `/v1/responses/compact`，仍仅在 endpoint 路径着色并保留详情原文。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name） | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes）        |
| ------------ | ------------ | ------------- | -------------- | ------------------------ | --------------- | ------------------- | -------------------- |
| None         | None         | internal      | None           | None                     | web             | Dashboard / Live    | 不新增或修改公开接口 |

## 验收标准（Acceptance Criteria）

- Given Dashboard 或 Live 的桌面请求表包含 `ivan-hkl-vless-vision-01KFXRNYWYXKN4JHCF3CCV78GD`，When 页面渲染完成，Then 代理名称在 badge 内被省略，完整文本仅通过 `title` 暴露。
- Given 同一场景，When 观察第二行代理列、model 列与展开按钮，Then 代理 badge 的右边界不超过 model 列左边界 1px，且整表 `overflowDelta <= 1`。
- Given compact 记录，When 检查摘要列表与详情，Then compact 只在 endpoint 路径呈现 info 样式，代理列不出现额外 compact badge。
- Given 移动端列表，When 展开 compact 记录详情，Then endpoint 原文 `/v1/responses/compact` 仍可见，且代理名称相关修复不影响现有展开字段。

## 实现前置条件（Definition of Ready / Preconditions）

- 根因已确认来自桌面代理列的额外 `flex-wrap` 包裹层，而非后端数据缺失。
- 回归边界已锁定为前端展示 hotfix，不涉及 API / 数据库 / pricing 改动。
- 长代理名样例与发布标签已冻结，可直接进入实现与快车道验证。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: `cd web && npm run test -- --run src/components/InvocationTable.test.tsx`
- E2E tests: `cd web && npm run test:e2e -- tests/e2e/invocation-table-layout.spec.ts`

### UI / Storybook (if applicable)

- Stories to add/update: `web/src/components/InvocationTable.stories.tsx`
- Visual regression baseline changes (if any): 无需新增 PR 图片；如需人工复核，可本地打开 Dashboard / Live 查看真实长代理名行。

### Quality checks

- Build / Storybook: `cd web && npm run build`、`cd web && npm run build-storybook`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增 hotfix spec 索引并同步状态/备注。
- `docs/specs/5gqdb-invocation-proxy-name-truncation-hotfix/SPEC.md`: 记录实现进度、验证结果与变更说明。

## 计划资产（Plan assets）

- Directory: `docs/specs/5gqdb-invocation-proxy-name-truncation-hotfix/assets/`
- In-plan references: 如需后续补图，使用 `![...](./assets/<file>.png)`
- PR visual evidence source: 本 hotfix 默认不要求新增 PR 图片。

## Visual Evidence (PR)

本次默认不放 PR 图片；若后续需要补充浏览器证据，只允许引用本目录 `./assets/` 下文件。

## 资产晋升（Asset promotion）

None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 修复桌面代理列 badge 宽度约束，恢复长代理名省略显示。
- [x] M2: 为 proxy badge/name 增加稳定测试选择器，并统一 Story/单测/E2E 的长代理名样例。
- [x] M3: 前端测试、build、storybook build 与 E2E 回归全部通过。
- [ ] M4: 快车道完成本地提交、PR、checks 与 review-loop 收敛。

## 方案概述（Approach, high-level）

- 以最小布局修复为主：删掉多余中间层，给桌面代理 badge 与文本补齐 `min-w-0` / `max-w-full` 约束，而不是重新设计整列结构。
- 让回归测试直接度量 DOM 几何关系，确保未来再改 compact/状态视觉时也能及时发现布局退化。
- 规格层保持 focused hotfix，只引用 `g3amk` 与 `r8m3k` 作为背景，不重复复制既有完整需求。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：若 badge 约束修得过窄，可能误伤短代理名与 `Direct` 的居中观感，需要用现有 fixture 回归确认。
- 风险：E2E 断言几何边界时需考虑浏览器亚像素误差，因此统一允许 1px 容差。
- 需要决策的问题：None。
- 假设（需主人确认）：None。

## 变更记录（Change log）

- 2026-03-09: 创建 hotfix spec，冻结根因、修复边界、回归样例与快车道发布口径。
- 2026-03-09: 已完成桌面代理列热修、长代理名回归断言与本地 `vitest/build/storybook/playwright` 验证，等待 PR/checks/review-loop 收敛。

## 参考（References）

- `docs/specs/g3amk-codex-remote-compact-observability/SPEC.md`
- `docs/specs/r8m3k-invocation-table-responsive-no-overflow/SPEC.md`
