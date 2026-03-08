# 请求侧 Fast 情报与中性闪电标识（#ww6et）

## 状态

- Status: 已完成（5/5）
- Created: 2026-03-08
- Last: 2026-03-08

## 背景 / 问题陈述

- 现有请求列表只展示响应侧 `serviceTier`，因此只能看出“实际命中了什么 tier”，无法看出“请求时是否明确想要 Fast / priority processing”。
- 当客户端请求 `service_tier=priority`，但上游实际返回 `service_tier=auto/default/flex` 或缺失时，当前 UI 没有任何区分，排障时会把“请求意图”和“响应结果”混在一起。
- 既有 proxy 观测链路已经保存了 request raw 文件和 payload JSON，具备从请求体补充 `requestedServiceTier` 的条件，无需新增 SQLite 列。

## 目标 / 非目标

### Goals

- 为 proxy 请求增加 `requestedServiceTier?: string`，仅从请求体顶层 `service_tier` / `serviceTier` 提取并统一归一化。
- 保留 `serviceTier` 继续表示响应实际生效 tier；HTTP `/api/invocations` 与 SSE `records` 同时返回两者。
- 将模型列 Fast 图标升级为三态：`effective`（响应命中 priority）、`requested_only`（请求想要 priority 但响应未命中）、`none`（其余）。
- 在详情区同时展示 `Requested service tier` 与 `Service tier`。
- 启动时尽力回填历史 proxy 记录的 `payload.requestedServiceTier`。

### Non-goals

- 不根据模型名、项目默认 tier、代理行为或任何启发式推断“请求想要 Fast”。
- 不为 quota 或其它非 proxy 来源伪造 `requestedServiceTier`。
- 不新增按 requested/effective tier 的聚合统计、筛选器或 SQLite schema 变更。

## 范围（Scope）

### In scope

- `src/main.rs`：请求侧 tier 提取、payload 写入、列表/SSE 投影、历史回填与回归测试。
- `web/src/lib/api.ts`、`web/src/lib/invocation.ts`、`web/src/components/InvocationTable.tsx`：消费 `requestedServiceTier`，渲染三态闪电与详情字段。
- `web/src/i18n/translations.ts`、`web/src/components/InvocationTable.stories.tsx`、Vitest 与 Playwright 场景更新。
- `docs/specs/README.md` 与本规格状态同步。

### Out of scope

- 数据库 schema 变更。
- Settings / Stats 页新增 requested/effective tier 聚合 UI。
- 任何依赖 CRS 或下游特定实现细节的推断逻辑。

## 需求（Requirements）

### MUST

- `requestedServiceTier` 仅接受请求体顶层 `service_tier` / `serviceTier` 的文本值，写入前必须 `trim + lowercase`。
- `GET /api/invocations` 与 SSE `records` 返回新增 `requestedServiceTier?: string`，字段名固定为 camelCase。
- Fast 图标三态口径固定：
  - `effective`: `serviceTier === 'priority'`
  - `requested_only`: `requestedServiceTier === 'priority' && serviceTier !== 'priority'`
  - `none`: 其它情况
- `effective` 继续使用现有琥珀色闪电；`requested_only` 使用中性色闪电；两者 tooltip / a11y 文案必须不同。
- 详情面板新增 `Requested service tier` 字段；缺失时显示 `—`。
- 启动回填仅扫描 `source=proxy` 且 `request_raw_path IS NOT NULL` 的记录；缺文件、坏 JSON、字段缺失时只能跳过，不得阻断服务启动。

### SHOULD

- 回填策略保持幂等，只补缺失 `payload.requestedServiceTier` 的记录。
- 前端测试与 Storybook 统一通过 `data-fast-state="effective|requested_only"` 断言图标语义，避免只测“是否有图标”。
- Dashboard 与 Live 由于共用 `InvocationTable`，应自然获得一致行为，不新增额外页面分支。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- proxy 请求进入系统时，从请求 JSON 顶层提取 `requestedServiceTier`，与 `promptCacheKey`、`reasoningEffort` 一起进入 payload summary。
- 请求完成后，payload 同时保存 `requestedServiceTier` 与响应侧 `serviceTier`；`/api/invocations` 与 SSE `records` 原样投影这两个字段。
- InvocationTable 根据 `requestedServiceTier` 与 `serviceTier` 计算三态闪电：
  - `priority -> priority`：琥珀色闪电；
  - `priority -> 非 priority/缺失`：中性色闪电；
  - `非 priority/缺失 -> priority`：仍视为实际命中，显示琥珀色闪电；
  - 其余不显示图标。
- 启动阶段回填会从 request raw 文件读取历史 proxy 请求体，补写缺失的 `payload.requestedServiceTier`。

### Edge cases / errors

- `auto`、`default`、`flex`、`scale`、空字符串、非字符串都允许保真返回，但只有 `priority` 会点亮图标状态。
- 若请求 raw 文件不存在、JSON 非法或顶层无该字段，历史记录保持缺失。
- 非 proxy 来源的 `requestedServiceTier` 必须保持缺失，不从 `serviceTier` 回填。

## 接口契约（Interfaces & Contracts）

| 接口（Name）                         | 类型（Kind）         | 范围（Scope） | 变更（Change） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes）                               |
| ------------------------------------ | -------------------- | ------------- | -------------- | --------------- | ------------------- | ------------------------------------------- |
| `GET /api/invocations` record object | HTTP API             | internal      | Modify         | backend         | web dashboard/live  | 新增 `requestedServiceTier?: string`        |
| `events` SSE `records` payload       | Event                | internal      | Modify         | backend         | web dashboard/live  | 与 HTTP 列表同构新增 `requestedServiceTier` |
| `ApiInvocation`                      | TypeScript interface | internal      | Modify         | web             | InvocationTable     | 新增 `requestedServiceTier?: string`        |

## 验收标准（Acceptance Criteria）

- Given 请求体顶层 `service_tier=priority`，When 请求完成并查询 `/api/invocations`，Then 返回 `requestedServiceTier: "priority"`。
- Given `requestedServiceTier === 'priority'` 且 `serviceTier !== 'priority'`，When 渲染模型列，Then 显示中性色闪电并带独立 tooltip / a11y 文案。
- Given `serviceTier === 'priority'`，When 渲染模型列，Then 继续显示现有琥珀色闪电，不受 `requestedServiceTier` 影响。
- Given 展开详情面板，When 字段存在或缺失，Then `Requested service tier` 与 `Service tier` 都可见，缺失值为 `—`。
- Given 历史 proxy 记录存在 `request_raw_path` 且 payload 缺 `requestedServiceTier`，When 服务启动执行回填，Then 能从 request raw 解析的记录被补齐，且再次启动不重复更新已完成记录。

## 实现前置条件（Definition of Ready / Preconditions）

- `requestedServiceTier` 的唯一来源和判定口径已冻结为“请求体顶层 `service_tier/serviceTier` 原值归一化”。
- 三态闪电的颜色与语义已冻结：琥珀色表示实际命中，中性色表示请求想要但未命中。
- 历史回填范围已冻结为 proxy + request raw 文件存在的记录。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust tests：覆盖请求侧 tier 提取、payload summary 写入、列表/SSE 投影与历史回填幂等。
- Vitest：覆盖三态闪电 helper 与详情字段渲染。
- Playwright：覆盖 `priority/priority`、`priority/auto`、`priority/缺失`、`auto/priority`、`flex/*` 至少一组表格/列表断言。
- Storybook：补齐 `effective`、`requested_only`、`none` 三类示例记录。

## 方案概述（Approach, high-level）

- 继续复用 `payload` JSON 作为轻量扩展点，避免 schema 迁移。
- 后端复用现有“从 request raw 回填上下文字段”的模式，为 `requestedServiceTier` 增加一条并行回填链路。
- 前端把“请求意图”和“实际命中”拆开表达：详情区字段保真展示，图标只承载三态摘要语义。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：历史 request raw 文件留存不完整时，回填覆盖率有限，缺失记录会继续显示 `—`。
- 开放问题：None。
- 假设：请求侧 `priority` 是唯一需要视觉高亮的“想要 Fast”意图，其它 tier 仅做字段保真展示。

## 里程碑（Milestones）

- [x] M1: docs/specs 新规格建档并在索引登记。
- [x] M2: 后端新增 `requestedServiceTier` 采集、投影与历史回填。
- [x] M3: InvocationTable 完成三态闪电与详情字段展示。
- [x] M4: Rust、Vitest、前端构建与 Playwright 回归通过。
- [x] M5: 快车道交付完成（commit / push / PR / checks / review-loop 收敛）。

## 变更记录（Change log）

- 2026-03-08: 创建规格，冻结请求侧 tier 口径、三态闪电语义与历史回填范围。
- 2026-03-08: 完成后端请求侧 tier 采集/回填与前端三态闪电落地，验证通过（M1-M4）。
- 2026-03-08: PR #95 checks 全绿，review-loop 收敛，无新增阻塞项，规格收口完成（M5）。
