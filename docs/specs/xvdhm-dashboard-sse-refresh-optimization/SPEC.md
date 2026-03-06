# Dashboard SSE 更新链路优化（#xvdhm）

## 状态

- Status: 已完成（6/6）
- Created: 2026-03-07
- Last: 2026-03-07

## 背景 / 问题陈述

- Dashboard 当前复用单一 `/events` SSE 通道，但总览页多个共享 hook 对同一批事件的处理策略不一致。
- scheduler 与 proxy capture 会重复计算/广播 summary 与 quota，即使当前无 SSE 订阅者，或 payload 与上次广播完全一致。
- `today` 等 calendar window 仍依赖“收到其他 summary 后最多 60 秒一次”的 fallback HTTP 刷新，导致总览页主要卡片与 records 的实时性不一致。
- `UsageCalendar` 的 `90d / 1d` timeseries 在收到 records 时会触发整段回源重算，容易形成无效请求风暴。
- `useTimeseries` 目前缺少 request sequencing、abort 与 in-flight dedupe，慢网/重连场景下存在重复请求与 stale response 覆盖风险。

## 目标 / 非目标

### Goals

- 保留现有公共 `/events`、`/api/stats/summary`、`/api/stats/timeseries` 接口与 payload schema 不变。
- 让 Dashboard 核心区块（recent records、`1d summary`、`24h/7d` 热力图、`today` 卡片）在新 records 入库后 1–2 秒内完成可见更新。
- 让 scheduler / proxy capture 在 `receiver_count == 0` 时跳过无意义的 summary/quota 计算与广播。
- 对 summary/quota 广播引入 changed-only 去重，避免重复 payload 触发前端白刷。
- 收敛共享 hooks 的刷新策略：calendar summary 走 records 驱动静默刷新，timeseries 走显式同步策略与请求去重。

### Non-goals

- 不新增 SSE event type、额外 SSE 通道或新的 HTTP route。
- 不改 Dashboard 视觉布局、文案结构或组件组合关系。
- 不把服务端改成按订阅者时区维持独立的 stateful calendar-window summary 广播。
- 不做全站状态管理重构，不引入新的全局 store 框架。

## 范围（Scope）

### In scope

- `src/main.rs` 中 scheduler / proxy-capture 的 summary/quota 广播链路。
- `web/src/hooks/useStats.ts`
- `web/src/hooks/useTimeseries.ts`
- `web/src/hooks/useInvocations.ts`（仅必要兼容调整）
- 相关 Rust / Vitest 测试。
- `docs/specs/README.md`

### Out of scope

- 页面组件树与视觉样式改造。
- 新增 quota 概览在 Dashboard 的展示。
- `Stats` / `Live` 页面额外产品行为 redesign。

## 需求（Requirements）

### MUST

- scheduler 在无 SSE 订阅者时仍执行 poll/persist，但不得计算或广播 summary/quota。
- proxy capture 路径在 records 广播后，仅当 summary/quota payload 与最近一次已广播值不同才发送对应事件。
- `today` / `thisWeek` / `thisMonth` 的 summary 刷新必须由 `records` 驱动，使用静默模式与 1 秒节流，不得在初次 hydration 后反复闪烁 loading。
- `useTimeseries` 必须具备 request sequencing、abort/cancel、in-flight dedupe 与 stale response 抑制能力。
- `90d / 1d` UsageCalendar 在同一本地日内收到 records 时，只允许本地修补当前日 bucket；全量回源只能发生在 reconnect/open、页面回前台或跨日校准时。
- `preferServerAggregation=true` 的 timeseries 仍以服务端结果为准，但 records 触发的回源必须节流（3 秒）且不会并发风暴。

### SHOULD

- 对未变化的 summary/quota payload 不触发前端 state update。
- 为关键节流/同步 helper 提供可测试的纯函数或稳定行为边界。
- 浏览器验证中确认 Dashboard Network 面板不再出现每条 records 都触发的 `range=90d&bucket=1d` timeseries 请求。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- scheduler tick：
  - 持续完成现有 poll / persist。
  - 若 `receiver_count == 0`，直接跳过 summary/quota 计算与广播。
  - 若有订阅者，仅广播 changed-only 的 summary/quota payload。
- proxy capture 新增记录：
  - 立即广播 `records`。
  - 复用 changed-only 逻辑尝试广播 summary/quota。
- Dashboard summary：
  - `1d` 直接消费 SSE `summary(window=1d)`。
  - `today/thisWeek/thisMonth` 消费 `records` 触发的 1 秒节流静默回源，并在 SSE `open` 后做一次静默补拉。
- Dashboard timeseries：
  - `1d/1m` 与 `7d/1h` 继续基于 records 本地增量更新。
  - `90d/1d` 仅修补当前本地日 bucket；历史 bucket 保持原值，依赖 reconnect/回前台/跨日校准时的全量回源纠正。
  - `preferServerAggregation=true` 继续走服务端聚合，但 records/open 只触发节流后的静默回源。

### Edge cases / errors

- summary/quota 计算失败仅记录 `warn`，不得影响 records 广播或主请求。
- 若同一时间存在 in-flight timeseries 请求，后续刷新必须复用或排队，不得无界叠加并发。
- 若旧请求晚于新请求返回，旧响应不得覆盖新状态。
- 若页面在后台，非强制刷新场景下不主动触发 records-driven 回源；页面恢复可见后再补拉一次。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name）                             | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers）      | 备注（Notes）                      |
| ---------------------------------------- | ------------ | ------------- | -------------- | ------------------------ | --------------- | ------------------------ | ---------------------------------- |
| `/events` payload schema                 | Event        | external      | Modify         | None                     | backend         | web dashboard/live/stats | 仅调整发送条件，不改 payload shape |
| `useSummary(window)` behavior            | Hook         | internal      | Modify         | None                     | web             | dashboard/stats/live     | 保持调用签名不变                   |
| `useTimeseries(range, options)` behavior | Hook         | internal      | Modify         | None                     | web             | dashboard/stats          | 保持调用签名不变                   |

### 契约文档（按 Kind 拆分）

None

## 验收标准（Acceptance Criteria）

- Given `/dashboard` 处于打开状态，When 新 invocation 入库并通过 SSE 收到 records，Then recent table 立即更新，且 `1d summary`、`24h` 热力图、`7d` 热力图在 1–2 秒内反映变化。
- Given Dashboard 显示 `today` 卡片，When 高频 records 连续到达，Then summary 以静默 HTTP 刷新为主、频率不超过每 1 秒 1 次，且初次 hydration 后不出现 loading 闪烁。
- Given UsageCalendar 已挂载，When 同一本地日内连续收到 records，Then 不会为每条 records 触发一次完整 `range=90d&bucket=1d` timeseries 请求，但当天格子数值仍可见更新。
- Given SSE 断线后重连，When `open` 事件触发，Then Dashboard 各相关数据执行一次静默 backfill/resync，并与后端状态收敛。
- Given scheduler tick 时没有 SSE 订阅者，When poll 正常完成，Then 仍完成持久化，但不会执行 summary/quota 计算与广播。
- Given summary/quota payload 与最近一次广播值一致，When scheduler 或 proxy capture 尝试广播，Then 不发送重复 `summary` / `quota` 事件。

## 实现前置条件（Definition of Ready / Preconditions）

- 目标、非目标与范围已冻结。
- Dashboard 近实时定义为“records 入库后 1–2 秒内核心区块可见更新”，而不是所有区块同帧同步。
- calendar windows 继续以浏览器时区配合现有 HTTP summary 计算，不新增时区态 SSE 契约。
- `90d / 1d` UsageCalendar 的历史 bucket 实时一致性依赖 reconnect/回前台/跨日校准，允许非同帧强一致。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust tests: 覆盖 no-subscriber skip 与 changed-only summary/quota broadcast。
- Unit tests: 覆盖 calendar summary 1 秒节流、timeseries request sequencing / stale suppression / no-storm 行为。
- E2E / browser check: 验证 Dashboard 在真实浏览器中的网络请求数量与重连补拉行为。

### Quality checks

- `cargo test`
- `cd web && npm run test`
- `cd web && npm run build`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增 #xvdhm 索引，并在实现推进后更新状态与 Notes。

## 计划资产（Plan assets）

- Directory: `docs/specs/xvdhm-dashboard-sse-refresh-optimization/assets/`
- PR visual evidence source: maintain `## Visual Evidence (PR)` in this spec when PR screenshots are needed.

## Visual Evidence (PR)

## 资产晋升（Asset promotion）

None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新建 spec 并登记 `docs/specs/README.md`。
- [x] M2: 后端 scheduler / proxy capture 增加 no-subscriber skip 与 changed-only summary/quota broadcast。
- [x] M3: `useSummary` / `useTimeseries` 收敛 dashboard 相关刷新策略并补请求去重保护。
- [x] M4: 补齐 Rust / Vitest 自动化回归。
- [x] M5: 完成浏览器验证并确认 Dashboard records 更新、no-storm 与 reconnect backfill 行为。
- [x] M6: 完成 fast-track PR、checks 跟踪与 review-loop 收敛。

## 方案概述（Approach, high-level）

- 后端通过内部缓存最近一次已发送的 summary/quota 值，避免重复 payload 触发无效前端刷新。
- 前端继续复用单一 SSE 通道，但把“何时回源、何时本地增量、何时忽略旧响应”的策略内聚到共享 hooks。
- Dashboard 只承接共享层行为收益，不新增页面级状态容器。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：`90d / 1d` 本地修补策略若边界处理错误，可能出现跨日时一天偏差；需用 reconnect/visibility/day-rollover 测试兜住。
- 风险：changed-only 比较若忽略浮点稳定性，可能因为序列化差异造成误判；应基于结构化值比较，而非 JSON 字符串文本。
- 假设：Dashboard 近实时优先级高于严格历史 bucket 同帧一致性。

## 变更记录（Change log）

- 2026-03-07: 创建规格并冻结优化边界、验收标准与快车道交付目标。
- 2026-03-07: 完成后端 changed-only 广播与前端 summary/timeseries 刷新策略收敛，`cargo test`、`cd web && npm test`、`cd web && npm run build` 通过。
- 2026-03-07: 浏览器实测 `/dashboard`，确认 records 推送后 recent table / `today` / `24h` / `7d` / `90d` 当前桶可见更新，且 reconnect 仅触发一轮静默 backfill 请求。
- 2026-03-07: 创建 PR #90，补齐 `type:patch` + `channel:stable` labels，并确认 Label Gate / CI Pipeline 全绿；review 复查未发现阻塞项。

## 参考（References）

- `docs/specs/5932d-sse-proxy-live-sync/SPEC.md`
- `docs/specs/rkc7k-live-summary-flicker-fix/SPEC.md`
- `docs/specs/rzxey-dashboard-usage-calendar-skeleton-shift/SPEC.md`
