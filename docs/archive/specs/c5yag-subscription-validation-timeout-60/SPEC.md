# Extend subscription validation timeout to 60s (keep single-proxy at 5s)（#c5yag）

## 状态

- Status: 已完成（3/3）
- Created: 2026-03-01
- Last: 2026-03-01

## 背景 / 问题陈述

- 设置页“添加订阅链接”在慢链路场景中频繁出现 `validation request timed out after 5s`。
- 现状是前后端都使用 5 秒超时，订阅验证流程容易误判失败。
- 订阅验证链路包含：拉取订阅、解析节点、代理探测上游，时延显著高于单条节点验证。

## 目标 / 非目标

### Goals

- 将“订阅 URL 验证”超时改为 60 秒，降低误超时。
- 保持“单条代理 URL 验证”超时为 5 秒，维持快速反馈。
- 前后端超时行为与错误文案秒数保持一致。

### Non-goals

- 不新增环境变量或配置项（本次采用硬编码常量）。
- 不修改 API 路径、请求/响应字段。
- 不修改订阅解析策略、可达性判定规则（2xx/401/403）。

## 范围（Scope）

### In scope

- `src/main.rs`: 订阅/单条验证超时分流（60s vs 5s）。
- `web/src/lib/api.ts`: 按 `kind` 分流 `AbortController` 超时（60_000ms vs 5_000ms）。
- 补充后端与前端测试，覆盖超时分流与超时消息秒数。

### Out of scope

- 设置页视觉与交互流程改版。
- 订阅探测节点数量、xray 运行机制变更。

## 接口契约（Interfaces & Contracts）

- 保持 `POST /api/settings/forward-proxy/validate` 不变：
  - request: `{ kind: 'proxyUrl' | 'subscriptionUrl', value: string }`
  - response: `{ ok, message, normalizedValue, discoveredNodes, latencyMs }`

## 验收标准（Acceptance Criteria）

- 订阅验证在 5~60 秒内完成时，不再出现 `...timed out after 5s`。
- 订阅验证超过 60 秒时，报错秒数应为 60。
- 单条代理验证超过 5 秒时，仍报 `...timed out after 5s`。
- 相关 Rust/Web 测试通过，且前端 build 通过。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 后端超时策略按验证类型分流（订阅 60s、单条 5s）
- [x] M2: 前端超时策略按 `kind` 分流并保持报错秒数一致
- [x] M3: 补齐测试并完成本地验证
