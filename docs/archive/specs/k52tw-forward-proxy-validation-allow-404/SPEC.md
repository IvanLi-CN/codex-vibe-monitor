# Forward proxy validation allows 404 as reachable（#k52tw）

## 状态

- Status: 已完成（3/3）
- Created: 2026-03-01
- Last: 2026-03-01

## 背景 / 问题陈述

- 线上订阅验证流程在上游返回 `404 Not Found` 时会被判定为不可达，并导致“添加订阅链接”失败。
- 现有逻辑将可达状态限定为 `2xx/401/403`，对“链路可达但资源路径不存在”的场景过于严格。
- 同一可达性判定同时被 `proxyUrl` 与 `subscriptionUrl` 复用，导致两条路径都可能误失败。

## 目标 / 非目标

### Goals

- 将验证探测的“可达”判定扩展为：`2xx/401/403/404`。
- 覆盖 `proxyUrl` 与 `subscriptionUrl` 两种验证路径。
- 增加回归测试，防止再次因状态码判定导致误失败。

### Non-goals

- 不引入 API key 配置能力。
- 不修改 `POST /api/settings/forward-proxy/validate` 的请求/响应字段。
- 不修改订阅超时预算（保持单条 5s、订阅 60s）。
- 不处理 `openai_upstream_base_url` 的路径拼接策略。

## 范围（Scope）

### In scope

- `src/main.rs`：抽取统一可达状态判定 helper，并在探测逻辑中复用。
- `src/main.rs`：新增状态判定与验证路径的 Rust 回归测试。
- `docs/specs/README.md`：新增本 spec 索引项并记录状态。

### Out of scope

- 前端 API 定义与请求参数改造。
- 新增环境开关控制 404 策略。

## 接口契约（Interfaces & Contracts）

- 保持 `POST /api/settings/forward-proxy/validate` 不变：
  - request: `{ kind: 'proxyUrl' | 'subscriptionUrl', value: string }`
  - response: `{ ok, message, normalizedValue, discoveredNodes, latencyMs }`

## 风险与取舍

- 允许 `404` 会把“网络可达但目标路径不存在”视为可达，这属于本次热修的明确取舍。
- 为降低过度放宽风险，`407/429/5xx` 仍保持失败。

## 验收标准（Acceptance Criteria）

- `proxyUrl` 验证在探测返回 `404` 时结果为成功。
- `subscriptionUrl` 验证在探测返回 `404` 时结果为成功。
- 探测返回 `5xx` 仍失败，错误信息包含状态码，保持可诊断性。
- 新增/调整的 Rust 测试通过。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 抽取并替换验证探测的可达状态判定（含 404）
- [x] M2: 补充状态判定单测（允许与拒绝两组）
- [x] M3: 补充 `proxyUrl/subscriptionUrl` 的 404 成功与 5xx 失败回归测试
