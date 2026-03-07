# 请求列表 Fast 模式标识（service tier 版）（#rw32e）

## 状态

- Status: 已完成（5/5）
- Created: 2026-03-07
- Last: 2026-03-07

## 背景 / 问题陈述

- 当前请求列表会展示模型名、代理、耗时与详情上下文，但无法直接判断某次请求是否实际命中了 OpenAI 的 fast / priority processing。
- 现有监控链路已经保存 `payload`、`raw_response` 与 proxy 原始响应文件路径，具备追加 `service_tier` 可观测性的基础，但 `/api/invocations` 和前端 `InvocationTable` 尚未投影该字段。
- 如果继续只靠模型名或外部经验推断 fast 模式，会把“请求意图”与“实际执行 tier”混为一谈，排障与计费观察都不可靠。

## 目标 / 非目标

### Goals

- 在不新增 SQLite 列的前提下，为 `/api/invocations.records[]` 增加 `serviceTier?: string`，值反映实际响应 `service_tier`。
- 仅当 `serviceTier === 'priority'` 时，在请求列表模型名后显示 Fast 闪电图标。
- 在请求详情区新增独立 `Service tier` 字段，并对历史记录做启动回填。

### Non-goals

- 不根据模型名、请求参数或任何启发式规则推断 fast 模式。
- 不把 `flex`、`default`、`auto` 或缺失值展示为 Fast。
- 不新增按 service tier 的筛选、排序、聚合统计或独立数据库 schema。

## 范围（Scope）

### In scope

- `src/main.rs`：采集实际 `service_tier`、写入 payload、列表投影、启动回填与相关测试。
- `web/src/lib/api.ts`、`web/src/lib/invocation.ts`、`web/src/components/InvocationTable.tsx`：消费 `serviceTier`，渲染 Fast 图标与详情字段。
- `web/src/i18n/translations.ts`、`web/src/components/InvocationTable.stories.tsx`、相关前端测试与 Playwright 回归。
- `docs/specs/README.md` 与本规格状态同步。

### Out of scope

- SQLite schema 变更。
- 基于 `*-spark`、`codex-spark` 或其它模型后缀的 fast 推断。
- Settings / Dashboard / Live 之外的新入口或额外统计卡片。

## 需求（Requirements）

### MUST

- `GET /api/invocations` 返回对象新增 `serviceTier?: string`，字段名固定为 camelCase。
- `serviceTier` 仅来源于实际响应 `service_tier`；当无法判定时保持缺失，不得伪造默认值。
- 模型名后的闪电图标仅在 `serviceTier === 'priority'` 时显示，图标需有 tooltip / a11y 文案。
- 详情面板新增 `Service tier` 字段；缺失时显示 `—`。
- 历史记录在启动阶段尽力回填 `payload.serviceTier`；损坏 JSON、缺失 raw 文件或无该字段时只能跳过，不得阻断服务启动。

### SHOULD

- HTTP 首屏列表与 SSE record 结构保持同构，避免首屏与增量记录展示不一致。
- 回填优先复用现有 `raw_response`，仅在 proxy 场景必要时回退到 `response_raw_path` 原始文件。
- `serviceTier='priority'` 的视觉标识不应破坏现有表格截断、对齐与响应式布局。

### COULD

- Storybook 示例与前端 fixture 同步覆盖 `priority`、`flex`、缺失三类记录，便于视觉回归。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- proxy 请求完成后，后端从实际响应 JSON / SSE 事件中提取 `service_tier`，写入 `payload.serviceTier`，并通过 `/api/invocations` 暴露为 `serviceTier`。
- XY 配额拉取记录如果上游返回 `serviceTier` / `service_tier`，后端同样写入 `payload.serviceTier`，供列表和详情统一消费。
- 用户在 Dashboard / Live 查看请求列表时：
  - 当 `serviceTier === 'priority'`，模型名后显示闪电图标；
  - 当 `serviceTier` 为其它值或缺失，不显示 Fast 图标；
  - 展开详情时总能看到 `Service tier` 字段。
- 服务启动时会批量扫描旧记录，为能从 `raw_response` 或 proxy 原始响应恢复出实际 tier 的记录补写 `payload.serviceTier`。

### Edge cases / errors

- `flex`、`default`、`auto`、空字符串、非字符串或畸形 payload 都不显示 Fast 图标。
- stream 响应只要任一事件给出实际 `service_tier`，即可作为该请求的 `serviceTier`；若整条流都没有，则字段保持缺失。
- 历史回填遇到缺失文件、非法 JSON、截断预览缺少该字段时，记录跳过并保持当前 payload，不视为失败。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name）                         | 类型（Kind）         | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes）                      |
| ------------------------------------ | -------------------- | ------------- | -------------- | ------------------------ | --------------- | ------------------- | ---------------------------------- |
| `GET /api/invocations` record object | HTTP API             | internal      | Modify         | None                     | backend         | web dashboard/live  | 新增 `serviceTier?: string`        |
| `events` SSE `records` payload       | Event                | internal      | Modify         | None                     | backend         | web dashboard/live  | 与 HTTP 列表同构新增 `serviceTier` |
| `ApiInvocation`                      | TypeScript interface | internal      | Modify         | None                     | web             | InvocationTable     | 新增 `serviceTier?: string`        |

### 契约文档（按 Kind 拆分）

- None

## 验收标准（Acceptance Criteria）

- Given 某次请求的实际响应 `service_tier=priority`，When 调用 `/api/invocations`，Then 返回记录包含 `serviceTier: "priority"`。
- Given 列表记录 `serviceTier === 'priority'`，When 渲染模型列，Then 模型名后显示闪电图标，并带有 Fast / priority 语义的 tooltip 与 a11y 文案。
- Given 记录 `serviceTier` 为 `flex`、`default`、`auto`、空值或缺失，When 渲染列表，Then 不显示 Fast 图标。
- Given 用户展开详情，When 记录包含 `serviceTier`，Then 显示 `Service tier` 字段和值；缺失时显示 `—`。
- Given 存在历史 proxy 记录且 `raw_response` 或 `response_raw_path` 可解析出实际 `service_tier`，When 服务启动执行回填，Then `payload.serviceTier` 被补齐且后续 `/api/invocations` 可返回该值。
- Given 回填遇到损坏 JSON、缺失文件或无 `service_tier`，When 启动继续执行，Then 该记录被跳过且服务启动不失败。

## 实现前置条件（Definition of Ready / Preconditions）

- `serviceTier` 的唯一语义已冻结为“实际响应 `service_tier` 原值”。
- Fast 图标判定口径已冻结为“仅 `priority` 点亮”。
- 历史记录需尽力回填且仍保持“不新增 SQLite 列”的实现边界。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: Rust 覆盖非流 / 流响应 `service_tier` 提取、`list_invocations` 字段投影与历史回填。
- Integration tests: proxy capture / XY record 持久化后 payload 能携带 `serviceTier`。
- E2E tests (if applicable): invocation table 的 Playwright 回归验证 priority 图标显示、详情字段存在、flex 不点亮。

### UI / Storybook (if applicable)

- Stories to add/update: `InvocationTable.stories.tsx` 覆盖 `priority`、`flex`、缺失三类记录。
- Visual regression baseline changes (if any): None。

### Quality checks

- `cargo test`
- `cargo check`
- `cd web && npm run test`
- `cd web && npm run build`
- `cd web && npm run test:e2e -- invocation-table-layout.spec.ts`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增规格索引并同步状态。

## 计划资产（Plan assets）

- Directory: `docs/specs/rw32e-invocation-fast-mode-indicator/assets/`
- In-plan references: None

## 资产晋升（Asset promotion）

- None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 后端采集实际 `service_tier`，通过 `/api/invocations` / SSE 暴露 `serviceTier`，并保持 payload-only 持久化。
- [x] M2: 启动回填补齐可恢复的历史 `payload.serviceTier`，跳过无法解析的记录。
- [x] M3: InvocationTable 模型列追加 Fast 闪电图标，详情区新增 `Service tier` 字段，并补齐 i18n / Storybook / fixture。
- [x] M4: Rust、Vitest、前端构建与 invocation table Playwright 回归全部通过。
- [x] M5: 完成 fast-track 交付（commit / push / PR / checks / review-loop / plan-sync）。

## 方案概述（Approach, high-level）

- 复用现有 `payload` JSON 作为轻量扩展点，把 `serviceTier` 与其他详情上下文字段保持同一路径管理，避免 schema 迁移。
- 后端统一提供“提取实际 `service_tier`” helper，覆盖普通 JSON、嵌套 `/response/service_tier` 与 SSE 逐事件扫描，减少采集 / 回填 / 投影三处逻辑漂移。
- 前端把 Fast 图标视为 `serviceTier === 'priority'` 的纯展示层语义：字段保真展示，图标单独判定，避免把其它 tier 误解释为 fast。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：部分历史 `raw_response` 只有截断预览，可能无法恢复 `service_tier`，这类记录会保持缺失。
- 需要决策的问题：None。
- 假设（需主人确认）：若 XY 上游未返回 `serviceTier/service_tier`，该来源记录允许保持缺失而不追加猜测值。

## 变更记录（Change log）

- 2026-03-07: 创建规格，冻结“仅实际 `service_tier=priority` 算 Fast”口径，并要求以 payload-only + 启动回填实现。
- 2026-03-07: 已完成后端 service tier 采集 / 回填、InvocationTable 图标与详情展示，以及 `cargo test`、`cargo check`、`cd web && npm run test`、`cd web && npm run build`、`cd web && npm run test:e2e -- invocation-table-layout.spec.ts` 验证。
- 2026-03-07: 已创建 PR #93，review-loop 发现并修复了 legacy `serviceTier=null` 时未回退 `service_tier` 的投影问题；合并 `main` 后重新推送，PR 已恢复 `mergeable_state=clean` 且 checks 全部通过。

## 参考（References）

- `docs/specs/hrvtt-invocation-proxy-weight-delta/SPEC.md`
- `docs/specs/26knq-invocation-table-overflow/SPEC.md`
