# OpenAI 兼容 WebSocket 代理（#w5s2x）

## 状态

- Status: active
- Created: 2026-05-04
- Last: 2026-05-04

## 背景 / 问题陈述

OpenAI Responses API 已公开 WebSocket mode，Codex 也已开始使用 WebSocket 承载多数 Responses API 流量。当前本服务的 OpenAI 兼容代理只处理 `/v1/*` HTTP 请求；若下游客户端切换为 WebSocket，本服务无法接入、路由到账号池或记录连接级状态。

## 目标 / 非目标

### Goals

- `/v1/*` 支持下游 WebSocket upgrade，并通过账号池选择上游账号。
- 提供全局配置开关，避免部署后默认改变 `/v1/*` 的兼容协议面。
- 将账号级 `upstreamBaseUrl` 的 `https/http` 映射为 `wss/ws`，再拼接原始 `/v1/*` path 与 query。
- WebSocket 帧透明双向转发，支持 text、binary、ping、pong、close。
- 记录连接级 pool attempt，并在终态后广播现有 pool attempt 快照。
- 保持现有 `/events` SSE 监控通道不变。

### Non-goals

- 不把监控 UI 的 SSE 改成 WebSocket。
- v1 不做 WebSocket 帧级 usage/cost 深解析。
- v1 不新增 SQLite schema。
- 已建立的 WebSocket 隧道中途断开后，v1 不做透明换号或帧级重放。

## 范围

### In scope

- `/v1/*` downstream WebSocket upgrade 检测、鉴权与账号池路由。
- 透明上游 WebSocket 连接与帧中继。
- downstream upgrade 前的上游账号连接 failover：上游 WS 握手失败时，代理记录失败、释放 reservation、排除该账号并按号池逻辑继续尝试其他候选，直到连上一个上游或耗尽 distinct-account retry budget。
- 连接级 pool attempt 观测、reservation 释放与 SSE 广播。

### Out of scope

- Dashboard/Live 前端实时订阅协议。
- Responses API WS 事件语义解析与 token 计费。
- 非 pool route 的直连反代恢复。

## 接口契约

- Downstream: `GET /v1/*` 携带标准 WebSocket upgrade headers，且必须携带现有 pool route key。
- Feature flag: `OPENAI_PROXY_WEBSOCKET_ENABLED=false` 时，WebSocket upgrade 返回 HTTP `503` JSON error；设置为 `true` 后才启用 WS 隧道。普通 HTTP proxy 不受该开关影响。
- Upstream URL:
  - account/global upstream base `https://host/base` -> `wss://host/base/<original-path>?<query>`
  - account/global upstream base `http://host/base` -> `ws://host/base/<original-path>?<query>`
- Headers:
  - 转发安全端到端 headers；
  - 不转发 hop-by-hop/upgrade headers；
  - API key 账号覆盖 `Authorization` 为账号配置；
  - OAuth 账号使用 `Bearer <access_token>`。
- Failure:
  - WebSocket support disabled：HTTP `503` JSON error，不建立上游连接。
  - pool route key 缺失或无效：HTTP `401` JSON error，和现有 HTTP proxy 行为一致。
  - 上游 URL 构造失败：HTTP `502` JSON error，记录失败且不重试该请求。
  - 单个上游 WS 连接或握手失败：记录 transport failure attempt，释放该账号 reservation，标记路由 transport failure，排除失败账号与 route key，并在同一个 downstream 请求内继续选择下一个账号。
  - 所有可用候选耗尽或达到 distinct-account retry budget：返回最后一次可重试失败对应的 HTTP error，或返回 pool 不可用错误。
  - 已建立隧道后的上游断开：关闭 downstream WebSocket 并记录终态；不在同一条已升级连接里换号。

## 验收标准

- Given 无有效 pool route key，When 请求 `/v1/responses` WebSocket upgrade，Then 返回 `401`，不建立上游连接。
- Given 有效账号与 mock upstream WS，When downstream 发送 text/binary/ping/close，Then upstream 收到对应帧，且 upstream 响应帧被转发给 downstream。
- Given 第一个上游账号 WS 连接失败且池内还有候选，When downstream 发起 upgrade，Then 代理在 downstream upgrade 前记录失败并切到下一个账号；若下一个账号成功，downstream 得到正常 WebSocket 隧道而不是先断开再依赖客户端重连。
- Given 所有上游 WS 连接失败，When downstream 发起 upgrade，Then downstream 收到 HTTP error，所有失败账号的 pool reservation 被释放，attempt 记录为 transport failure。
- Given account `upstreamBaseUrl=https://example.test/base`，When 下游连接 `/v1/responses?model=x`，Then 上游目标为 `wss://example.test/base/v1/responses?model=x`。

## Task Orchestration

- wave: 1
  - main-agent => 建立 WebSocket proxy spec 与接口边界 (skill: $docs-plan)
- wave: 2
  - main-agent => 实现 `/v1/*` WebSocket upgrade、上游连接、帧中继与 attempt 观测 (skill: $fast-flow)
- wave: 3
  - main-agent => 添加 Rust 覆盖并跑相关验证 (skill: $fast-flow)
- wave: 4
  - main-agent => review-loop、提交、push 与 PR 收敛 (skill: $fast-flow + $codex-review-loop)

## Assumptions

- WebSocket 连接为长连接交互，v1 的成本统计只记录连接级 metadata，不估算 token/cost。
- 现有 pool route key 是 WebSocket 下游鉴权入口，不新增独立 WS token。
- 代理只能在 downstream WebSocket upgrade 前可靠切换上游账号；upgrade 之后若要做到跨账号透明恢复，需要上游协议提供可验证的 resume/replay 语义，否则无法保证不丢帧、不重放或不破坏会话状态。
