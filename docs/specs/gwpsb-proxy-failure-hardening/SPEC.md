# 线上失败请求分类治理与可观测性增强（#gwpsb）

## 状态

- Status: 待实现
- Created: 2026-02-24
- Last: 2026-02-24

## 背景 / 问题陈述

- 当前失败统计把服务端故障、调用方错误、客户端中断混在一起，无法快速定位可行动问题。
- 线上最近 24h 存在大量 `downstream_closed`、`upstream_stream_error`、`failed_contact_upstream`、`request_body_read_timeout`。
- 需要在不破坏现有数据与接口兼容的前提下，新增失败分类字段并改造统计口径。

## 目标 / 非目标

### Goals

- 为 `codex_invocations` 增加 `failure_kind`、`failure_class`、`is_actionable` 字段并补齐历史数据。
- 新增可按失败范围过滤的错误分布接口与失败摘要接口。
- 前端统计页支持按范围筛选错误分布，并展示可行动失败率。
- 收敛默认超时参数，降低长尾阻塞时长。

### Non-goals

- 不调整 SSO、Traefik 或外层网关架构。
- 不引入流式请求自动重试。
- 不改动与失败治理无关的业务功能。

## 范围（Scope）

### In scope

- `src/main.rs` 中 schema、失败分类逻辑、统计接口与启动 backfill 流程。
- `web/src/**` 中错误分布 scope 切换与失败摘要展示。
- `docs/specs/**` 同步状态与验收信息。

### Out of scope

- 外部 relay 服务实现修改。
- 认证体系设计变更。

## 需求（Requirements）

### MUST

- 新增字段必须向后兼容，老数据不丢失。
- `downstream_closed` 必须归类为 `client_abort`，不计入可行动故障。
- `failed_contact_upstream`、`upstream_stream_error`、`request_body_read_timeout` 必须归类为 `service_failure` 且 `is_actionable=true`。
- `api key` 相关错误必须归类为 `client_failure`。
- 至少新增一条后端自动化测试覆盖分类逻辑。

### SHOULD

- 错误分布接口默认范围为 `service`，降低误报噪声。
- 新增失败摘要接口返回三类失败与可行动失败率。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 启动阶段执行 schema ensure，并对历史 `codex_invocations` 进行失败分类 backfill（幂等）。
- 新写入的调用记录在落库时同步写入 `failure_kind`、`failure_class`、`is_actionable`。
- `/api/stats/errors` 支持 `scope=all|service|client|abort`，默认 `service`。
- `/api/stats/failures/summary` 返回同一时间窗口下失败摘要和可行动失败率。
- 统计页支持 scope 切换，并显示失败摘要。

### Edge cases / errors

- 历史记录 `error_message` 为空时，分类结果应可回退到 `none` 或 `service_failure` 的保守策略，不得 panic。
- 旧记录没有新字段时，接口仍可返回（字段可为空或默认值）。

## 接口契约（Interfaces & Contracts）

- `GET /api/invocations` 新增字段：
  - `failureKind?: string`
  - `failureClass?: "service_failure" | "client_failure" | "client_abort" | "none"`
  - `isActionable?: boolean`
- `GET /api/stats/errors` 新增 query：
  - `scope=all|service|client|abort`（默认 `service`）
- 新增 `GET /api/stats/failures/summary`：
  - 返回 `serviceFailureCount`、`clientFailureCount`、`clientAbortCount`、`actionableFailureCount`、`actionableFailureRate`。

## 验收标准（Acceptance Criteria）

- Given 含有 `downstream_closed` 的记录，When 请求错误分布 `scope=service`，Then 该类记录不会出现在结果中。
- Given 含有 `failed_contact_upstream` 的记录，When 请求失败摘要，Then `serviceFailureCount` 与 `actionableFailureCount` 增加。
- Given 历史无分类字段数据，When 服务启动后查询调用列表，Then 返回的记录包含分类字段（允许少量未识别为 `none`）。
- Given 统计页切换 scope，When 数据刷新，Then 饼图按对应范围更新。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo test` 至少覆盖新增分类函数单测。
- `cd web && npm run test` 通过（至少覆盖受影响 hook/API 调用路径）。

### Quality checks

- `cargo fmt` 无差异。
- `cargo check` 与 `cd web && npm run build` 通过。

## 实现里程碑（Milestones / Delivery checklist）

- [ ] M1: schema 与分类逻辑落地，新增 backfill。
- [ ] M2: 错误分布 scope 与失败摘要接口落地。
- [ ] M3: 前端统计页接入 scope 与失败摘要。
- [ ] M4: 测试、验证、提交、PR 与收敛。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：历史错误字符串多样，规则匹配可能遗漏。
- 风险：启动 backfill 在大库上耗时增加。
- 开放问题：无。
- 假设：当前仓库是线上部署来源，新增字段可平滑上线。

## 变更记录（Change log）

- 2026-02-24: 初始化规格并冻结实现范围与验收标准。
