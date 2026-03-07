# InvocationTable 推理强度与详情 reasoningTokens（#rupn7）

## 状态

- Status: 已完成
- Created: 2026-03-07
- Last: 2026-03-07

## 背景 / 问题陈述

- 当前请求记录列表已经具备 `reasoningTokens` 数据链路，但列表与展开详情都没有展示该字段，导致用户无法直接判断响应里是否发生了推理 token 消耗。
- 请求侧的推理强度（Responses 的 `reasoning.effort` / Chat Completions 的 `reasoning_effort`）尚未被采集，列表里缺少“这次调用以什么推理强度发起”的上下文。
- 若不补齐这两项信息，排查成本与模型行为观察仍需要回看原始请求/响应，列表页证据不足。

## 目标 / 非目标

### Goals

- 在不改 SQLite schema 的前提下，为请求记录补充 `reasoningEffort?: string` 字段投影。
- 在 InvocationTable 主列表显示推理强度，在展开详情显示推理强度与 `reasoningTokens`。
- 对历史 proxy 记录提供基于 `request_raw_path` 的 best-effort backfill，补齐缺失的推理强度。

### Non-goals

- 不推断模型默认推理强度。
- 不把 `reasoningTokens` 升级为主列表独立列。
- 不改动成本统计、图表聚合或 quota/dashboard 视图。

## 范围（Scope）

### In scope

- 后端在 `prepare_target_request_body` 提取 Responses `reasoning.effort` 与 Chat Completions `reasoning_effort`。
- payload summary、新记录查询与 SSE records 统一增加 `reasoningEffort`。
- 启动期新增 raw request 文件回填：仅补 payload 缺失的 `reasoningEffort`。
- 前端列表与详情展示补齐推理强度、详情补齐 `reasoningTokens`。

### Out of scope

- 数据库新列或迁移。
- 无原始请求文件的历史数据修复。
- 新增独立 API、筛选器或统计卡片。

## 需求（Requirements）

### MUST

- `GET /api/invocations` record object 新增 `reasoningEffort?: string`。
- SSE `records` payload 与 HTTP 列表字段保持同构。
- 主列表在桌面与移动断点都能展示推理强度。
- 展开详情必须展示 `reasoningEffort` 与 `reasoningTokens`，无值回退为 `—`。

### SHOULD

- 历史回填复用现有 raw-file backfill 模式，不引入 schema 变更。
- 未知推理强度值按原样显示，不因前端枚举不全而丢失。

### COULD

- Storybook 示例覆盖 Responses/Chat Completions 两种强度口径。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 新的 `/v1/responses` 请求带 `reasoning.effort` 时，记录列表显示该值；展开详情同时显示该值与响应里的 `reasoningTokens`。
- 新的 `/v1/chat/completions` 请求带 `reasoning_effort` 时，记录列表显示该值；若响应无 `reasoningTokens`，详情显示 `—`。
- 应用启动时扫描 payload 缺失 `reasoningEffort` 且 `request_raw_path` 可读的 proxy 记录，并从原始请求 JSON 回填。

### Edge cases / errors

- 请求未显式设置推理强度时，列表与详情显示 `—`。
- 原始请求文件不存在、已过期或 JSON 非法时，回填跳过该记录并继续执行。
- payload 非法 JSON 时，列表接口继续容错返回，`reasoningEffort` 为空。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name）                         | 类型（Kind）         | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers）  | 备注（Notes）                                   |
| ------------------------------------ | -------------------- | ------------- | -------------- | ------------------------ | --------------- | -------------------- | ----------------------------------------------- |
| `GET /api/invocations` record object | HTTP API             | internal      | Modify         | None                     | backend         | web dashboard/live   | 新增 `reasoningEffort?: string`                 |
| `events` SSE `records` payload       | Event                | internal      | Modify         | None                     | backend         | web dashboard/live   | 与 HTTP 列表同构新增 `reasoningEffort`          |
| `ApiInvocation` (web)                | TypeScript interface | internal      | Modify         | None                     | web             | InvocationTable      | 新增 `reasoningEffort?: string`                 |
| `InvocationTable` details view       | UI contract          | internal      | Modify         | None                     | web             | dashboard/live users | 详情新增 `reasoningEffort` 与 `reasoningTokens` |

### 契约文档（按 Kind 拆分）

- None

## 验收标准（Acceptance Criteria）

- Given `/v1/responses` 请求体含 `{"reasoning":{"effort":"high"}}`，When 查询 `/api/invocations` 或接收 SSE records，Then `reasoningEffort` 返回 `high` 且主列表显示 `high`。
- Given 用户展开含 `reasoningTokens` 的记录详情，When 详情渲染，Then 可见 `reasoningTokens` 数值。
- Given 请求未设置推理强度，When 渲染列表与详情，Then 推理强度显示 `—`。
- Given 历史 proxy 记录 payload 缺 `reasoningEffort` 且 `request_raw_path` 可读，When 服务启动完成，Then payload 被回填并在列表查询中可见。
- Given 原始请求文件缺失或非法 JSON，When 启动回填执行，Then 服务不中断且该记录保持空值。

## 实现前置条件（Definition of Ready / Preconditions）

- 范围与验收口径已冻结（主列表展示推理强度，详情展示推理强度与 reasoningTokens）。
- 历史回填口径已冻结为“仅 raw request 可读时 best-effort 回填”。
- 前后端字段命名统一为 `reasoningEffort`。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: Rust 请求体解析、`list_invocations` 投影与 malformed payload 容错回归。
- Integration tests: 启动期 reasoningEffort backfill 回填成功/失败路径。
- E2E tests (if applicable): InvocationTable 列表与详情展示推理强度/`reasoningTokens`。

### UI / Storybook (if applicable)

- Stories to add/update: `InvocationTable.stories.tsx` 补充 `reasoningEffort` 与详情 `reasoningTokens` 场景。
- Visual regression baseline changes (if any): None.

### Quality checks

- `cargo fmt --check`
- `cargo test`
- `cargo check`
- `cd web && npm run test`
- `cd web && npm run build`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增本 spec 索引并更新状态。

## 计划资产（Plan assets）

- Directory: `docs/specs/rupn7-invocation-table-reasoning-effort/assets/`
- In-plan references: None

## 资产晋升（Asset promotion）

- None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 后端提取并投影 `reasoningEffort`，新记录 payload summary 与 `/api/invocations`/SSE 一致暴露。
- [x] M2: 启动期 raw request backfill 补齐缺失 `reasoningEffort`。
- [x] M3: InvocationTable 主列表显示推理强度，展开详情显示推理强度与 `reasoningTokens`。
- [x] M4: 补齐后端/前端测试并通过质量门槛。

## 方案概述（Approach, high-level）

- 复用现有 payload JSON 承载新增字段，避免 schema 变更与迁移成本。
- 将推理强度提取集中在请求体解析阶段，避免多条 capture 分支重复解析。
- 回填沿用现有 prompt cache key backfill 的 raw-file 扫描模式，保持容错一致性。
- 前端继续保持响应式表格结构，只在现有列内增加第二行信息，避免表格再次膨胀。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：不同 endpoint 的请求体字段形状存在差异，需要保证提取逻辑互不误伤。
- 需要决策的问题：None.
- 假设（需主人确认）：未知推理强度值直接原样展示即可满足观察诉求。

## 变更记录（Change log）

- 2026-03-07: 初始化规格，冻结“主列表展示推理强度 + 详情展示推理强度与 reasoningTokens + 历史 raw-file best-effort 回填”口径。
- 2026-03-07: 完成后端 `reasoningEffort` 采集/回填、InvocationTable 展示与回归测试，并通过 PR #92 提交。

## 参考（References）

- `docs/specs/hrvtt-invocation-proxy-weight-delta/SPEC.md`
- `docs/specs/r8m3k-invocation-table-responsive-no-overflow/SPEC.md`
