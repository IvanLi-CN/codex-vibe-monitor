# 请求详情补齐代理信息与本次权重变化（#hrvtt）

## 状态

- Status: 已完成
- Created: 2026-03-02
- Last: 2026-03-02

## 背景 / 问题陈述

- 当前请求详情虽然已展示基础上下文和阶段耗时，但缺少独立的“代理”字段，定位某次请求命中的代理节点不够直接。
- Forward Proxy 已在运行时维护权重并按调用结果调整，但调用详情中无法看到“本次调用对权重造成的变化”。
- 不补齐该信息会导致故障排查和权重策略观察依赖运行态面板，缺少逐请求证据链。

## 目标 / 非目标

### Goals

- 在不改 SQLite 表结构的前提下，为 `/api/invocations` 增加 `proxyWeightDelta` 可选字段。
- 在详情面板补齐“代理”与“代理权重变化（本次）”展示。
- 以“仅新记录生效”实现权重变化观测，历史记录保持回退显示。

### Non-goals

- 不做历史记录回填估算。
- 不调整 Forward Proxy 算法参数或惩罚/恢复策略。
- 不改动 Forward Proxy 设置页结构与交互。

## 范围（Scope）

### In scope

- 后端记录 `record_forward_proxy_attempt` 本次更新前后权重与 delta（`after - before`）。
- 代理 capture 路径将 `proxyWeightDelta` 写入 payload summary。
- `/api/invocations` 与 SSE records 回查统一投影 `payload.proxyWeightDelta`。
- 前端 InvocationTable 详情区新增“代理”“代理权重变化（本次）”。

### Out of scope

- 数据库 schema 变更。
- 历史数据批处理回填。
- 新增独立权重审计接口。

## 需求（Requirements）

### MUST

- `GET /api/invocations` 返回对象新增 `proxyWeightDelta?: number`。
- 详情区代理信息粒度固定为“仅代理名称（proxyDisplayName）”。
- 详情区权重变化固定为“仅Δ、带符号、两位小数”。
- 当无值/不可计算时前端回退 `—`，不得报错。

### SHOULD

- SSE `records` 与 HTTP 列表字段保持同构，避免页面首次加载与实时更新展示不一致。
- malformed payload 场景保持容错，字段为空而非请求失败。

### COULD

- Storybook 示例可补充 `proxyWeightDelta` 以便手动验证视觉展示。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 新的 `/v1/responses` 或 `/v1/chat/completions` 调用完成后，后端在记录 forward proxy 尝试时计算权重 delta，并随 payload 落库。
- 用户在 Dashboard/Live 展开任意调用详情时，看到：
  - 代理：命中代理展示名。
  - 代理权重变化（本次）：形如 `+0.55` / `-0.68`。

### Edge cases / errors

- 请求在读取 body 失败等前置阶段中断，未产生权重更新时，`proxyWeightDelta` 为空。
- 历史记录或 payload 缺失键时，详情显示 `—`。
- payload 非法 JSON 时，`/api/invocations` 仍正常返回，`proxyWeightDelta` 为空。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name）                         | 类型（Kind）         | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes）                    |
| ------------------------------------ | -------------------- | ------------- | -------------- | ------------------------ | --------------- | ------------------- | -------------------------------- |
| `GET /api/invocations` record object | HTTP API             | internal      | Modify         | None                     | backend         | web dashboard/live  | 新增 `proxyWeightDelta?: number` |
| `events` SSE `records` payload       | Event                | internal      | Modify         | None                     | backend         | web dashboard/live  | 通过同一回查 SQL 同步新增字段    |
| `ApiInvocation` (web)                | TypeScript interface | internal      | Modify         | None                     | web             | InvocationTable     | 新增 `proxyWeightDelta?: number` |

### 契约文档（按 Kind 拆分）

- None

## 验收标准（Acceptance Criteria）

- Given 新调用命中 forward proxy，When 查询 `/api/invocations`，Then 记录对象可返回 `proxyWeightDelta` 数值。
- Given 用户展开详情，When 记录包含权重变化，Then 显示“代理权重变化（本次）”且格式为带符号两位小数。
- Given 历史记录或不可计算场景，When 展开详情，Then 字段显示 `—`。
- Given payload 为 malformed JSON，When 请求 `/api/invocations`，Then 接口成功返回且 `proxyWeightDelta` 为空。
- Given SSE 推送新 records，When 前端渲染详情，Then 与 HTTP 首屏字段一致。

## 实现前置条件（Definition of Ready / Preconditions）

- 范围与验收口径已冻结（最简代理信息 + 仅Δ + 历史不回填）。
- 后端权重变化计算位置固定在 `record_forward_proxy_attempt`。
- 前后端字段命名统一为 `proxyWeightDelta`。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: Rust `list_invocations` 投影回归 + malformed payload 容错回归。
- Integration tests: 代理 capture 路径 payload 包含 `proxyWeightDelta`。
- E2E tests (if applicable): InvocationTable 展开详情时可见权重变化字段。

### UI / Storybook (if applicable)

- Stories to add/update: `InvocationTable.stories.tsx` 示例记录补充 `proxyWeightDelta`。
- Visual regression baseline changes (if any): None。

### Quality checks

- `cargo fmt --check`
- `cargo test`
- `cargo check`
- `cd web && npm run test`
- `cd web && npm run build`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增本 spec 索引并更新状态。

## 计划资产（Plan assets）

- Directory: `docs/specs/hrvtt-invocation-proxy-weight-delta/assets/`
- In-plan references: None

## 资产晋升（Asset promotion）

- None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 后端记录权重变化并写入 payload summary。
- [x] M2: `/api/invocations` 与 SSE records 投影新增 `proxyWeightDelta`。
- [x] M3: InvocationTable 详情区新增代理与权重变化展示（仅Δ）。
- [x] M4: 补齐后端与前端回归测试并通过质量门槛。

## 方案概述（Approach, high-level）

- 在 forward proxy attempt 记录函数内集中计算权重前后值，避免分散在各 capture 分支重复实现。
- 继续复用 payload JSON 承载新增字段，保持 schema 稳定。
- 前端格式化逻辑集中在组件函数中，统一处理符号、保留位数与空值回退。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：不同失败路径是否都触发权重更新，需要回归覆盖确保一致。
- 需要决策的问题：None。
- 假设（需主人确认）：仅展示代理名称即可满足“代理信息缺失”诉求。

## 变更记录（Change log）

- 2026-03-02: 初始化规格，冻结“最简代理信息 + 仅Δ + 历史不回填”口径。
- 2026-03-02: 完成后端字段投影与详情展示改造，新增回归测试并通过 `cargo` 与 `web` 质量检查。

## 参考（References）

- `docs/specs/z9h7v-invocation-log-observability/SPEC.md`
- `docs/specs/r8m3k-invocation-table-responsive-no-overflow/SPEC.md`
