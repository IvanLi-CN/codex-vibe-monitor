# OAuth 数据面内联合并（#pd77h）

## 状态

- Status: 待实现
- Created: 2026-03-16
- Last: 2026-03-16

## 背景 / 问题陈述

- 当前 OAuth 数据面通过固定 sidecar `ai-openai-oauth-bridge` 提供 `/openai/v1/*` 兼容层，主服务在 pool 路由前还要额外走一次内部 token register。
- 这一层抽象没有给本项目带来可复用收益，却把部署从单服务抬成了双进程/双容器，现网已经出现“重新授权成功，但因 sidecar 未启动而被重新打成异常”的故障。
- 主服务与 sidecar 实际上都在同一仓库、同一镜像里，继续保留自调用 HTTP 边界只会增加错误面、文档复杂度和排障成本。

## 目标 / 非目标

### Goals

- 把 OAuth Codex 数据面适配逻辑内联到主进程，删除 sidecar 生命周期、内部 register/token_key 语义与固定服务名依赖。
- 保持对外 `/api/pool/*`、`/v1/*` 访问方式稳定，避免影响现有前端和下游客户端。
- 把 pool 路由成功响应抽象升级为传输无关形态，使 API Key 直连和 OAuth 内联都能走同一消费链。
- 保留当前 OAuth refresh / usage sync / 显式失效转 `needs_reauth` 的规则，不把 bridge 故障语义继续带入新实现。

### Non-goals

- 不新增新的 OAuth provider、账号级 OAuth 数据面配置项或用户侧 env。
- 不改变 API Key 账号的 upstream 覆盖逻辑、pool 选路策略与 forward proxy 通路。
- 不保留 `openai-oauth-bridge` 兼容入口或过渡版本。

## 范围（Scope）

### In scope

- 主进程内 OAuth Codex adapter 与 pool 内部响应抽象重构。
- 删除 sidecar 二进制、镜像双入口和相关部署/排障文案。
- 同步 Rust / Web / docs / tests，使“单服务 OAuth 数据面”成为唯一口径。

### Out of scope

- 改写 OpenAI pool API 对外 wire shape。
- 新增与本次内联无关的权限模型、限流策略或 UI 功能。

## 需求（Requirements）

### MUST

- OAuth pool 路由不得再访问 `http://ai-openai-oauth-bridge:3000/internal/token/register` 或 `http://ai-openai-oauth-bridge:3000/openai`。
- `PoolResolvedAccount` 与 pool 请求发送链路必须能显式区分 API Key 直连与 OAuth 内联执行，不能再把 OAuth 伪装成“Bearer bridge token + 本地 HTTP upstream”。
- pool 成功响应必须升级为传输无关的统一抽象，能够同时承载 reqwest 上游响应与主进程内构造的 OAuth adapter 响应。
- OAuth `/v1/models`、`/v1/responses`、`/v1/responses/compact`、`/v1/chat/completions` 的兼容行为必须保留：模型列表归一化、非 stream `/responses` 的 SSE 收敛、stream passthrough 与错误摘要规则都不能回退。
- 只有 refresh/token 明确失效时才能把账号标成 `needs_reauth`；数据面可用性/权限/上游拒绝必须落 `error`，且不再写入 bridge register 相关错误。
- Docker 镜像与部署文档必须回到单服务口径，只保留 `codex-vibe-monitor` 可执行文件。

### SHOULD

- 历史 bridge 错误字符串在 UI 详情里允许继续只读展示，直到该账号下一次同步或路由结果覆盖为止。
- 内联后的 OAuth adapter 测试注入点应尽量复用现有本地 test server 模式，避免整体重写测试夹具。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- OAuth 账号同步：仍先刷新 access token、再抓 usage snapshot；该流程不依赖任何本地 sidecar。
- OAuth pool 路由：主进程直接根据请求路径调用内联 adapter，adapter 使用当前 access token 调用 `https://chatgpt.com/backend-api/codex` 并产出 OpenAI-compatible 响应。
- API Key pool 路由：继续使用现有 `OPENAI_UPSTREAM_BASE_URL` / 账号级 `upstreamBaseUrl` 直连 reqwest 请求路径。
- pool 消费链：不再要求上游一定是 `reqwest::Response`；统一从“状态码 + headers + body stream”抽象读取首包、透传 headers、记录 timings 与错误。

### Edge cases / errors

- OAuth 数据面返回明确 `invalid_grant` / token invalidated / must sign in again：账号转 `needs_reauth`。
- OAuth 数据面返回普通 `401/403/5xx/429`、结构化错误或 transport 失败：账号转 `error`，保留可诊断错误摘要。
- 非 stream `/v1/responses` 若上游 SSE 没有 `response.completed` 或出现 `response.failed`：按现有错误摘要规则返回 `error`。
- 大 body 或 live body replay 路径下，OAuth 请求允许先等待 replay snapshot 完整后再进入内联 adapter，不要求保留旧 sidecar 的“边读边转发”假象。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name） | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes） |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `/api/pool/upstream-accounts*` | HTTP API | external | Modify | None | backend | web | 仅错误口径与文案变化，对外路径不变 |
| `/v1/*` pool routing | HTTP API | external | Modify | None | backend | downstream clients | 对外路径不变，内部执行形态改为单进程 |
| `PoolResolvedAccount` / pool upstream response | internal | internal | Modify | None | backend | proxy / pool router | 明确 transport 分流并去除 reqwest-only 假设 |

### 契约文档（按 Kind 拆分）

None

## 验收标准（Acceptance Criteria）

- Given 单服务部署的新版本
  When 只运行 `codex-vibe-monitor`
  Then OAuth 账号同步和 `/v1/*` pool 路由都可工作，仓库文档不再要求 `ai-openai-oauth-bridge`。

- Given OAuth 账号重新授权成功
  When 后续被真实 `/v1/responses` 请求选中
  Then 不会再因为内部 register 端点不可达而进入 `error`，且 `last_error` 不再出现 `failed to contact oauth bridge token register endpoint`。

- Given API Key 与 OAuth 账号混合在 pool 中
  When 请求经过 pool failover / sticky / replay 流程
  Then API Key 路径行为保持不变，OAuth 路径通过内联 adapter 返回兼容响应，统一消费链记录 timings 与错误。

- Given OAuth 数据面出现明确 refresh/token 失效
  When 请求或同步触发该失效
  Then 账号状态仍会转成 `needs_reauth`，而不是模糊地写成普通 `error`。

## 实现前置条件（Definition of Ready / Preconditions）

- 目标/非目标、单进程边界与“无过渡兼容”策略已锁定
- `/v1/*` 对外兼容范围已确认
- pool 内部传输抽象已明确，不再要求实现阶段自行决定响应类型

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: OAuth adapter 请求改写、模型列表归一化、SSE completed/error 提取、错误摘要。
- Integration tests: pool OAuth route 的 invalid_grant / token invalidated / 一次 stale token 恢复 / 单服务路径。
- E2E tests (if applicable): 线上等价流程的账号详情错误口径与重新授权后的路由表现。

### Quality checks

- `cargo check`
- `cargo test`
- `cd web && bun run test`
- `cd web && bun run build`
- 单镜像容器 smoke（只启动 `codex-vibe-monitor`）

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新 spec 入索引，旧 sidecar spec 改为重新设计
- `docs/specs/u8j4n-fixed-oauth-bridge-sidecar/SPEC.md`: 标记被本 spec 取代
- `README.md`: 删除 sidecar 双服务说明，改成单服务 OAuth 数据面说明
- `docs/deployment.md`: 删除 sidecar 部署与排障步骤，改成主进程内联语义

## 计划资产（Plan assets）

- Directory: `docs/specs/pd77h-oauth-inline-adapter/assets/`
- In-plan references: `![...](./assets/<file>.png)`
- PR visual evidence source: maintain `## Visual Evidence (PR)` in this spec when PR screenshots are needed.

## Visual Evidence (PR)

## 资产晋升（Asset promotion）

None

## 实现里程碑（Milestones / Delivery checklist）

- [ ] M1: 规格与部署口径切换到单进程 OAuth adapter
- [ ] M2: 后端内联 OAuth adapter + pool 响应抽象完成
- [ ] M3: Web 文案与测试迁移到新口径
- [ ] M4: 快车道验证、PR、checks、review-loop 收敛完成

## 方案概述（Approach, high-level）

- 复用现有 `src/oauth_bridge.rs` 的数据面适配逻辑，但删除 server/register 生命周期，只保留“直接拿 access token 调 codex backend 并生成兼容响应”的能力。
- pool 路由在账号解析阶段显式区分 API Key 与 OAuth，两条路径在成功后统一收敛到传输无关响应抽象。
- 保持现有 refresh/sync/状态机规则，只删除 sidecar 专属缓存与恢复分支。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：pool live body replay 路径在 OAuth 内联后需要完整 snapshot 才能改写 `/responses` 请求。
- 风险：统一响应抽象涉及 proxy capture / stream 转发链，若改动不完整容易造成首包或 header 透传回归。
- 假设（需主人确认）：本次允许删除 `openai-oauth-bridge` 二进制与全部 sidecar 文档，不保留兼容入口。

## 变更记录（Change log）

- 2026-03-16: 创建替代 spec，定义用单进程 OAuth adapter 取代固定 sidecar。

## 参考（References）

- `docs/specs/u8j4n-fixed-oauth-bridge-sidecar/SPEC.md`
- `src/oauth_bridge.rs`
- `src/upstream_accounts/mod.rs`
- `src/main.rs`
