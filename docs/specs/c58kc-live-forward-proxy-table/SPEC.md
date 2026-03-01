# 实况页新增“代理”统计表与 24h 成败示意图（#c58kc）

## 状态

- Status: 部分完成（4/5）
- Created: 2026-03-01
- Last: 2026-03-01

## 背景 / 问题陈述

- `Live` 页面当前只有汇总卡片、实时图表与最新请求列表，缺少“按代理节点”的运行态观察。
- 设置页虽然有代理窗口统计，但不是实况视角，且没有 24h 成功/失败走势示意。
- 不补齐该视图会导致排障和节点对比依赖人工切页与日志，效率低且易误判。

## 目标 / 非目标

### Goals

- 在 `Live` 页面新增代理统计表，按节点展示与设置页一致的 `1m/15m/1h/1d/7d` 统计口径。
- 每个节点新增 24h 成功/失败示意图（固定 1h 粒度，24 桶）与总成功/失败次数。
- 后端提供只读聚合接口，避免实况页拉取完整 `/api/settings`。
- 保持当前持久化结构不变，复用 `forward_proxy_attempts`。

### Non-goals

- 不改代理调度权重算法与路由策略。
- 不新增 SSE 事件类型。
- 不改设置页交互与保存流程。
- 不引入数据库迁移。

## 范围（Scope）

### In scope

- Rust 后端新增 `GET /api/stats/forward-proxy`。
- 前端新增 API 类型、数据 hook、实况代理表格组件。
- `Live` 页面接入新 section（位于 summary 与 live chart 之间）。
- 增补 Rust/前端测试与 i18n 文案。

### Out of scope

- Dashboard/Stats 页面重构。
- 代理验证流程改造。
- 运行时配置项扩展（例如图表粒度可配置）。

## 需求（Requirements）

### MUST

- 24h 统计固定返回 24 个小时桶，缺失数据补零。
- 行维度覆盖运行时节点（含 direct），按 `displayName` 排序。
- 5 个窗口统计口径与设置页一致（attempts/successRate/avgLatencyMs）。
- `Live` 页数据刷新策略为：首屏拉取 + SSE `records` 触发节流刷新 + 60s 兜底刷新。
- 空数据和错误态需稳定可见，不影响 `Live` 现有其它区块。

### SHOULD

- 24h 示意图保持低视觉噪音，单行可快速比较代理健康度。
- 组件在移动端可横向滚动且不遮挡关键信息。

### COULD

- 后续在 tooltip 中补充单桶详细值（本轮非必需）。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 打开 `Live` 页面时，代理表格自动加载并展示每个节点 5 个窗口统计 + 24h 成败示意图。
- 收到 SSE `records` 事件时，代理表执行节流刷新（最短 5 秒一次）。
- 若 SSE 长时间无事件，60 秒轮询兜底刷新一次。

### Edge cases / errors

- 后端返回空节点时，前端显示空态文案，不报错。
- 接口失败时仅在代理表区域显示错误，不阻断实况页其它模块。
- 某节点 24h 无数据时，示意图展示全零条并显示 `S:0 / F:0`。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name）                   | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes）          |
| ------------------------------ | ------------ | ------------- | -------------- | ------------------------ | --------------- | ------------------- | ---------------------- |
| `GET /api/stats/forward-proxy` | HTTP API     | internal      | New            | None                     | backend         | web/live            | 只读聚合，无写入副作用 |

### 契约文档（按 Kind 拆分）

- None

## 验收标准（Acceptance Criteria）

- Given 已有运行时代理节点，When 打开 `Live` 页面，Then 页面显示“代理”表格并包含每行 `1m/15m/1h/1d/7d` 统计值。
- Given 任一代理节点，When 查看 24h 列，Then 可见 24 个 1h 桶的成功/失败示意图与总成功/失败次数。
- Given 某小时没有该代理尝试记录，When 查询 24h 数据，Then 对应桶返回 `successCount=0` 且 `failureCount=0`。
- Given 接口异常，When 页面渲染，Then 代理区块显示错误提示且 `Live` 其它区块可正常使用。
- Given 事件持续到达，When SSE `records` 触发刷新，Then 代理区块刷新频率不超过每 5 秒 1 次。

## 实现前置条件（Definition of Ready / Preconditions）

- 接口结构、字段命名和 24h 粒度已冻结。
- UI 位置和信息层级已确定（summary 与 chart 之间）。
- 验收口径覆盖 core path、空态和异常态。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: Rust 新增接口聚合与补零逻辑测试。
- Integration tests: Rust handler 返回结构与 direct 节点覆盖。
- Front-end tests: Vitest 覆盖 API/hook/组件关键渲染分支。

### UI / Storybook (if applicable)

- Stories to add/update: 本轮可不新增（可选）。
- Visual regression baseline changes: None。

### Quality checks

- `cargo fmt`
- `cargo check`
- `cargo test`
- `cd web && npm run test`
- `cd web && npm run build`

## 文档更新（Docs to Update）

- `docs/specs/README.md`：新增 spec 索引并同步状态。
- `docs/specs/c58kc-live-forward-proxy-table/SPEC.md`：随实现进度更新里程碑与状态。

## 计划资产（Plan assets）

- Directory: `docs/specs/c58kc-live-forward-proxy-table/assets/`
- In-plan references: None

## 资产晋升（Asset promotion）

- None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新增后端 `/api/stats/forward-proxy`，返回节点窗口统计与 24h 小时桶。
- [x] M2: 补齐后端测试，覆盖聚合、补零与空节点语义。
- [x] M3: 前端新增 API + hook + `ForwardProxyLiveTable` 并接入 `Live` 页面。
- [x] M4: 完成 i18n 文案与移动端可读性处理。
- [ ] M5: 完成本地验证与 fast-track 交付收敛（PR + checks + review-loop）。

## 方案概述（Approach, high-level）

- 复用现有 forward proxy runtime 快照与窗口统计查询，新增 24h 小时桶聚合查询。
- 前端采用独立 hook 解耦 `Live` 页面现有数据流，避免对 `useSettings` 的耦合和额外负载。
- 图形表现采用轻量 mini bars，不引入额外图表库或复杂交互。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：高频刷新导致无意义请求放大；通过 5s 节流 + 60s 兜底控制。
- 风险：小时桶边界理解偏差；统一使用 UTC 小时对齐并输出 ISO 时间避免歧义。
- 假设：`forward_proxy_attempts.occurred_at` 使用 UTC，可与 SQLite `strftime('%s', occurred_at)`一致换算。

## 变更记录（Change log）

- 2026-03-01: 新建规格，冻结字段口径与验收标准。
- 2026-03-01: 完成后端接口、前端页面接线与本地验证，状态更新为 `部分完成（4/5）`。

## 参考（References）

- `web/src/pages/Live.tsx`
- `web/src/pages/Settings.tsx`
- `src/main.rs`（forward proxy runtime + attempts）
