# Compact 502 可追踪性与号池动态超时（#4reae）

## 状态

- Status: 进行中（1/5）
- Created: 2026-03-24
- Last: 2026-03-24

## 背景 / 问题陈述

- 线上 `/v1/responses/compact` 失败时最终经常统一表现为 `502`，但返回体里没有可直接回查的 `invoke_id`，导致记录页和 pool attempts 很难关联。
- 当前号池路由设置只持久化 API key，和请求路径直接相关的代理/号池超时仍固定来自进程启动配置，线上调优必须重启。
- compact 请求是否被上游账号支持，目前只能靠离线排查错误文本，没有账号级观测状态，也无法在 UI 上快速判断“明确不支持”与“只是暂时超时/5xx”的差异。

## 目标 / 非目标

### Goals

- 所有代理错误响应都带上可查询的 `cvmId`，直接复用当前请求的 `invoke_id`，并保持现有 `error` 字段兼容。
- 将请求链路使用的关键超时迁移到号池模块动态维护，默认 compact handshake timeout 提升到 `300s`，更新后无需重启即可生效。
- 为 compact 请求引入账号级被动观测：成功标记 `supported`，明确能力负信号标记 `unsupported`，超时/transport/泛化 5xx 只记录本次观测事实，不参与路由限制。
- 在账号池列表、详情和路由设置弹窗里展示 compact 观测与超时设置。

### Non-goals

- 不基于 compact 支持观测做路由过滤、摘号、权重调整或 cooldown。
- 不改变 pool distinct-account budget、429 聚合、502/429 终态选择等既有失败语义。
- 不把动态超时扩展到 forward-proxy 验证、订阅探测、后台维护任务等非请求链路。

## 范围（Scope）

### In scope

- `src/main.rs` 代理失败响应、capture invoke id 贯穿、请求时超时解析、compact 观测判定与 pool attempts 持久化。
- `src/upstream_accounts/mod.rs` 号池路由设置 schema/API、compact 账号观测落库与列表/详情输出。
- `src/api/mod.rs` pool attempts API 输出 compact 观测字段。
- `web/src/lib/api.ts`、`web/src/pages/account-pool/UpstreamAccounts.tsx`、相关组件/测试与翻译。
- `docs/specs/README.md` 与 README 中 compact timeout 说明。

### Out of scope

- `/v1/responses` 与 `/v1/responses/compact` 的重试预算或 timeout-route-failover 策略本身。
- 新增第二个设置入口或新增独立“超时设置页”。
- 对历史 attempt 数据做离线回填；新字段仅对新请求生效。

## 需求（Requirements）

### MUST

- 代理错误响应保持扁平 JSON：至少包含现有 `error` 字段，并新增 `cvmId`；同时响应头必须带 `X-CVM-Invoke-Id`。
- `/api/pool/routing-settings` 的 GET/PUT 必须返回和保存一组动态超时：
  - `defaultFirstByteTimeoutSecs`
  - `responsesFirstByteTimeoutSecs`
  - `upstreamHandshakeTimeoutSecs`
  - `compactUpstreamHandshakeTimeoutSecs`
  - `requestReadTimeoutSecs`
- compact handshake timeout 默认值必须从 `180` 提升到 `300` 秒；其他默认值沿用当前配置。
- 对 `/v1/responses/compact`：
  - 成功请求必须把账号观测为 `supported`
  - 明确能力负信号必须把账号观测为 `unsupported`
  - transport 失败、timeout、泛化 `5xx/502/503/524` 不得把账号观测降级为 `unsupported`
- pool upstream attempt 记录必须新增 compact 观测字段，支持通过 `invoke_id` 回看每次尝试的判定。

### SHOULD

- 缺失的 timeout 持久化字段应在首次加载时用当前 `AppConfig` 默认值补齐，避免 rollout 期间出现 `NULL` 配置。
- UI 在号池列表中应直接暴露 compact 支持状态，在详情页应补充最近观测时间与原因。
- routing settings 弹窗应允许只修改 timeout 而不强制重新填写 API key。

## 功能与行为规格（Functional/Behavior Spec）

### 代理错误契约

- `proxy_openai_v1_common` 在进入请求链路前生成本次 `invoke_id`，并将其复用于 capture 持久化。
- 当代理最终返回错误响应时：
  - body 返回 `{ error, cvmId }`
  - header 返回 `X-CVM-Invoke-Id`
- 当前 pool 终态错误选择逻辑保持不变；即预算耗尽仍可统一返回 `502`，但 `cvmId` 必须能关联到 invocation / pool attempts。

### 动态超时

- 号池路由设置表新增 timeout 字段；读取时若发现为空，使用当前 `AppConfig` 的运行时值补齐并持久化。
- 请求链路在每次处理时都从持久化设置解析 timeout，并应用到：
  - request body read timeout
  - 通用 upstream handshake timeout
  - compact upstream handshake timeout
  - 通用 first-byte timeout
  - `/v1/responses` first-byte timeout
- 其余不在请求链路内的超时继续使用原有配置，不接入本次动态设置。

### compact 支持观测

- 观测只在 `/v1/responses/compact` 上生效。
- 观测结果分为：
  - `unknown`
  - `supported`
  - `unsupported`
- `supported`/`unsupported` 会回写到 `pool_upstream_accounts` 的最新观测字段。
- `unknown` 只写入本次 attempt 记录，不覆盖账号级既有观测状态。
- 明确能力负信号基于错误文本/错误码判定，例如 “no available channel for model ... compact” 或明确的 unsupported endpoint/model 文本。

## 验收标准（Acceptance Criteria）

- 任意 pool routing 触发的代理 `502` 都返回相同的 `invoke_id` 到 `X-CVM-Invoke-Id` 与 JSON `cvmId`，且可用该 ID 查询 invocation detail 与 pool attempts。
- 客户端若仅依赖 `error` 字段，行为保持兼容，不需要升级。
- 在账号池路由设置里修改 timeout 后，后续请求立即使用新值，无需重启服务。
- 新装/升级环境在未显式保存 timeout 前，`compactUpstreamHandshakeTimeoutSecs` 默认按 `300` 生效。
- compact 成功会把账号状态标记为 `supported`；显式 unsupported/no-channel-for-compact 类错误会标记为 `unsupported`；timeout/transport/泛化 5xx 不会把账号能力改成 `unsupported`。
- 账号池列表与详情页能看到 compact 支持状态、最近观测时间与原因；pool attempts API 能看到每次尝试的 compact 观测结果。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo test pool_routing_settings_`
- `cargo test pool_route_compact_`
- `cargo test proxy_openai_v1_`
- `cd web && bun run test -- UpstreamAccounts`
- `cd web && bun run test -- api.test.ts`

### Quality checks

- `cargo fmt --all`
- `cargo check`
- `cd web && bun run test`

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/4reae-compact-502-traceability-dynamic-pool-timeouts/SPEC.md`
- `README.md`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 冻结错误契约、compact 观测规则、动态 timeout schema 与 UI 入口。
- [ ] M2: 完成后端 schema、动态 timeout、生效路径与 `cvmId` 透传。
- [ ] M3: 完成 compact 观测落库、API 输出与记录页关联字段。
- [ ] M4: 完成账号池 UI 展示与路由设置弹窗编辑。
- [ ] M5: 完成 fast-track 收口到 merge-ready（验证、review-loop、PR）。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：若把动态 timeout 读取扩散到非请求链路，会意外改变后台维护语义；本次必须严守边界。
- 风险：compact 能力负信号判断过宽会把暂时性 `503` 误标为 `unsupported`；判定必须保守。
- 假设：既有 invocation detail / pool attempts 查询路径已经足够满足 `cvmId` 排查，不需要额外新建查询接口。

## 变更记录（Change log）

- 2026-03-24: 创建 spec，冻结 `cvmId` 错误契约、compact 被动观测与号池动态 timeout 的范围和验收。
