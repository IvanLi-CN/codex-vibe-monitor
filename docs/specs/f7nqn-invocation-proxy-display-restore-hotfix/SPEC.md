# InvocationTable 代理节点展示恢复热修（#f7nqn)

## 状态

- Status: 进行中
- Created: 2026-03-17
- Last: 2026-03-17

## 背景 / 问题陈述

- 生产 `Dashboard / Live` 的 InvocationTable 已经切成“账号 / 代理”双行摘要，但 2026-03-17 的新请求里 `proxyDisplayName` 持续为空，导致列表第二行与详情里的“代理”字段都显示 `—`。
- 同一时间生产 `/api/settings` 里的 `forwardProxy.nodes[].displayName` 正常存在，说明不是代理节点配置丢失，而是 invocation 采集链路没有稳定写入展示值。
- 对 `routeMode=pool` 的新记录，当前成功路径没有真正的 `SelectedForwardProxy`，但仍然需要给前端一个可读的“代理节点”展示值，否则账号列改造后会把空洞直接暴露出来。

## 目标 / 非目标

### Goals

- 恢复未来新产生 invocation 的 `proxyDisplayName`，让列表摘要与详情重新显示可读代理节点信息。
- 保持 `/api/invocations` 与前端 `ApiInvocation` 契约不变，只修 payload 填充逻辑。
- 为 pool 路由与 forward proxy 路由都补齐回归保护，避免长位置参数再次错位。

### Non-goals

- 不回填历史空值记录；旧数据仍允许显示 `—`。
- 不回滚账号列、时延列或当前页账号抽屉功能。
- 不新增新的公开 API 字段或数据库迁移。

## 范围（Scope）

### In scope

- `src/main.rs` 的 invocation payload 构造与 `proxyDisplayName` 解析策略。
- `src/tests/mod.rs` 的后端回归测试。
- `web/src/components/InvocationTable.test.tsx` 的前端展示断言。
- `docs/specs/README.md` 与本热修 spec 的同步。

### Out of scope

- 历史记录 backfill。
- 其它业务表格与号池页面布局调整。
- 生产代理配置本身的修改。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- forward proxy 路由继续优先使用 `SelectedForwardProxy.display_name` 作为 `proxyDisplayName`。
- pool 路由在没有 `SelectedForwardProxy` 时，改为从 `PoolResolvedAccount.upstream_base_url` 派生展示值：优先显示 host，存在端口时显示 `host:port`。
- invocation payload 构造改成具名 `ProxyPayloadSummary`，把 `routeMode`、`upstreamAccount*`、`responseContentEncoding`、`proxyDisplayName` 统一集中写入，避免再被长位置参数错位污染。
- 前端继续沿用现有回退：新记录应展示非空代理节点名，历史空值仍显示 `—`。

### Edge cases / errors

- OAuth 号池账号的上游地址默认会落到 `https://chatgpt.com/backend-api/codex`，列表展示值应归一成 `chatgpt.com`。
- API key 号池账号若上游地址包含端口，展示值必须保留端口，便于区分测试/内网 relay。
- 在请求读取阶段尚未选中账号或代理时，`proxyDisplayName` 仍可为空，不伪造值。

## 验收标准（Acceptance Criteria）

- Given 一条新的 `routeMode=pool` invocation，When 它被写入并从 `/api/invocations` 读回，Then `proxyDisplayName` 为非空且等于该账号 `upstream_base_url` 的 host 或 `host:port`。
- Given 一条新的 `routeMode=forward_proxy` invocation，When 它被写入并从 `/api/invocations` 读回，Then `proxyDisplayName` 保持选中 forward proxy 的 display name。
- Given 前端收到非空 `proxyDisplayName`，When 渲染摘要与展开详情，Then 两处都展示相同代理名。
- Given 生产发布完成后新产生一条 invocation，When 在生产容器内查询 `/api/invocations?limit=20`，Then 最新记录的 `proxyDisplayName` 不再为 `null`。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo check`
- `cargo test resolve_invocation_proxy_display_name`
- `cargo test pool_route_switches_accounts_after_first_chunk_failures_are_exhausted`
- `cargo test list_invocations_projects_payload_context_fields`
- `cd web && bun run test -- --run src/components/InvocationTable.test.tsx`
- `cd web && bun run build`

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/f7nqn-invocation-proxy-display-restore-hotfix/SPEC.md`

## 风险 / 假设

- 风险：pool 路由的“代理”展示值本质上是上游 host，而不是真实前置 relay 节点名；这是当前生产链路里唯一稳定可得的非空代理上下文。
- 假设：主人接受“只修未来，不回填历史”的 hotfix 边界。
