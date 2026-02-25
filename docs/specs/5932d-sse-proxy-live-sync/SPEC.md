# SSE 驱动的请求记录与统计实时更新（#5932d）

## 状态

- Status: 待实现
- Created: 2026-02-25
- Last: 2026-02-25

## 背景 / 问题陈述

- 前端 `Dashboard` 与 `Live` 已订阅 SSE，但当前代理链路写库成功后不会即时推送 `records`/`summary`/`quota`。
- 后端 `records` 广播仅来自轮询任务，导致代理请求产生后 UI 需要等待下一轮轮询或手动刷新。
- 目标是让代理请求写库后 <1s 在 UI 可见，并且统计卡片与配额快照同步刷新。

## 目标 / 非目标

### Goals

- 代理请求写库成功后，立即广播 `records`（仅新增记录）。
- 同步广播最新 `summary`（既有窗口集合）与 `quota` 快照。
- 前端在 SSE 重连后做一次静默回源，补齐重连窗口内可能丢失的记录。
- 保持现有 HTTP API 与 SSE payload schema 不变。

### Non-goals

- 不新增 SSE event type。
- 不改动 Dashboard/Live 组件接口与页面结构。
- 不引入 schema migration。

## 范围（Scope）

### In scope

- `src/main.rs`：代理落库路径与广播逻辑抽取。
- `web/src/hooks/useInvocations.ts`：SSE open 后的静默回源补齐。
- 相关测试与验证命令更新（Rust + Web）。

### Out of scope

- 轮询任务广播策略重构。
- 新增前端轮询兜底主机制。

## 需求（Requirements）

### MUST

- 新增内部 helper 统一“代理落库后广播”流程，替换代理链路的所有落库调用点。
- `INSERT OR IGNORE` 未插入时，不广播 `records`。
- 广播 `records`、`summary`、`quota` 之间错误隔离，任何广播失败仅记录 `warn`，不影响代理响应。
- 保持 `t_total_ms`/`t_persist_ms` 的现有更新语义，不得回归。

### SHOULD

- 复用 `collect_summary_snapshots` 与 `QuotaSnapshotResponse::fetch_latest`。
- 避免在多个错误/成功分支复制广播逻辑。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 代理请求落库成功且为新插入记录时：
  - 广播 `BroadcastPayload::Records { records: vec![record] }`
  - 广播全部 summary 窗口 payload
  - 广播最新 quota（存在时）
- 代理请求命中重复写入（未插入）时：
  - 不广播 `records`，并且不触发额外 summary/quota 广播。
- 前端连接 SSE 成功（open）后：
  - 对 `useInvocationStream` 执行一次静默 `fetchInvocations(...)` 回源并合并去重。

### Edge cases / errors

- summary 计算失败：记录 `warn`，继续执行 quota 广播尝试。
- quota 拉取失败：记录 `warn`，不影响请求主流程。
- SSE 广播通道拥塞/lag：记录 `warn`，代理请求照常返回。

## 接口契约（Interfaces & Contracts）

- HTTP API: 无变更。
- SSE schema: 保持现有 `records` / `summary` / `quota` / `version` 结构，不扩展字段与事件类型。

## 验收标准（Acceptance Criteria）

- Given 代理请求写库成功，When 订阅 `/events`，Then 在 1 秒内收到包含新增 `invokeId` 的 `records` 事件。
- Given 同一代理请求，When `records` 事件发送后，Then 能收到对应窗口的 `summary` 与最新 `quota` 事件。
- Given 命中 `INSERT OR IGNORE` 未插入，When 请求完成，Then 不重复发送 `records` 事件。
- Given SSE 发生断线并恢复，When 连接 open，Then 前端列表通过静默回源补齐，且与后端一致。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- 运行并通过与改动直接相关的 Rust 测试（覆盖代理落库后广播路径）。
- 运行并通过前端构建或测试校验（至少一种自动化验证）。

### Performance & Reliability

- 代理主链路不可因广播失败而失败。
- 不新增显著阻塞路径与重复广播噪声。

## 实现里程碑（Milestones / Delivery checklist）

- [ ] M1: 抽取落库后广播 helper，并改造 `persist_proxy_capture_record` 返回语义支持“是否新插入”。
- [ ] M2: 替换代理链路 5 处落库调用点为统一 helper。
- [ ] M3: 前端 `useInvocationStream` 增加 SSE open 后静默回源补齐。
- [ ] M4: 完成验证、提交、PR、checks 与 review-loop 收敛（fast-track）。

## 风险 / 假设

- 风险：summary/quota 查询在高频代理流量下增加读压；通过错误隔离和轻量查询控制影响。
- 假设：`invoke_id + occurred_at` 仍然可用于去重语义，不需要新增唯一键策略。
