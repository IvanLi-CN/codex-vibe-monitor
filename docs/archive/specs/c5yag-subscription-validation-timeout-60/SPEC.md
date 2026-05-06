# Forward proxy subscription validation scans all nodes concurrently（#c5yag）

## 状态

- Status: 已实现，待 PR / CI / review-proof 收敛
- Created: 2026-03-01
- Last: 2026-05-06

## 背景 / 问题陈述

- 设置页“添加订阅链接”在慢链路场景中频繁出现 `validation request timed out after 5s`。
- 初版修复将订阅验证预算扩展到 60 秒，但订阅节点探测仍只串行检查前 3 个节点。
- 订阅验证链路包含：拉取订阅、解析节点、代理探测上游，时延显著高于单条节点验证。
- 当前线上失败模式是：订阅本身可拉取且后续节点可用，但排序靠前的慢/坏节点占满验证预算，导致整体误判失败。

## 目标 / 非目标

### Goals

- 订阅 URL 验证必须扫描订阅解析出的全部 supported proxy entries，不再限制为前 3 个。
- 节点探测使用节点级并发上限 10；任意节点通过即可判定订阅可用。
- 每个节点最多探测 3 次；每次探测必须在 10 秒内从 upstream `/v1/models` 收到 HTTP status。
- 保持“单条代理 URL 验证”超时为 5 秒，维持快速反馈。
- 保持前端订阅验证整体等待预算为 60 秒，避免 UI 长时间悬挂。

### Non-goals

- 不新增环境变量或配置项（本次采用硬编码常量）。
- 不修改 API 路径、请求/响应字段。
- 不修改订阅解析策略、Xray outbound 构造或可达性判定规则（2xx/401/403/404）。

## 范围（Scope）

### In scope

- `src/main.rs`: 订阅验证并发/重试/单次探测超时常量。
- `src/forward_proxy/slices/storage_and_hourly_stats.rs`: 全量节点扫描、10 并发、每节点最多 3 次、单次 10 秒响应超时与早停成功逻辑。
- 后端测试覆盖慢前序节点、全量扫描、第 3 个以后节点可用、每节点最多 3 次。

### Out of scope

- 设置页视觉与交互流程改版。
- 前端订阅验证整体 60 秒等待预算改动。

## 接口契约（Interfaces & Contracts）

- 保持 `POST /api/settings/forward-proxy/validate` 不变：
  - request: `{ kind: 'proxyUrl' | 'subscriptionUrl', value: string }`
  - response: `{ ok, message, normalizedValue, discoveredNodes, latencyMs }`
- 成功响应中 `discoveredNodes` 表示订阅中 supported endpoint 总数，`latencyMs` 表示首个成功节点探测的耗时。
- 失败响应需说明已扫描节点数、并发上限、每节点重试次数和单次探测超时。

## 验收标准（Acceptance Criteria）

- 订阅中第 1 个节点慢/超时、第 2 个或更后节点可用时，验证应快速通过。
- 订阅中第 4 个或更后节点可用时，验证必须通过。
- 订阅中所有节点失败时，每个节点最多探测 3 次，单次探测预算为 10 秒。
- 任意节点返回 `2xx/401/403/404` 后，接口返回 `ok: true`，不等待其余节点全部完成。
- 单条代理验证超过 5 秒时，仍报 `...timed out after 5s`。
- 相关 Rust/Web API 测试通过。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 后端超时策略按验证类型分流（订阅 60s、单条 5s）
- [x] M2: 前端超时策略按 `kind` 分流并保持报错秒数一致
- [x] M3: 补齐历史超时测试并完成本地验证
- [x] M4: 订阅节点验证改为全量扫描 + 10 并发 + 每节点最多 3 次 + 单次 10 秒
- [x] M5: 补齐并发扫描与重试回归测试
