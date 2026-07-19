---
title: Main-app pure SSE topic subscriptions
module: web-dashboard
problem_type: architecture
component: Main-app realtime subscriptions
tags:
  - dashboard
  - sse
  - subscriptions
  - snapshot
  - replay
status: active
related_specs:
  - docs/specs/5932d-sse-proxy-live-sync/SPEC.md
  - docs/specs/z6ysw-dashboard-account-activity-tabs/SPEC.md
---

# Main-app pure SSE topic subscriptions

## Context

主应用当前态面板曾长期混用三种机制：

- `records` SSE 作为“有变化了”的通知，
- 页面各自的 HTTP bootstrap / open-resync / timer reconcile，
- 前端从 records、recent、timeseries 再拼出其它聚合面板。

这种设计把订阅 UI 变成多套真相源，恢复语义也无法统一。

## Symptoms

- 首屏先等 HTTP，再接 SSE，导致“当前态”并不真正由订阅驱动。
- 断线恢复后常常通过隐式 HTTP 回补，owner-facing 看起来像推送，实际上还是拉。
- 同一屏不同面板使用不同 cadence 与不同聚合来源，容易出现同屏口径漂移。

## Root Cause

根因不是 SSE 太弱，而是把 SSE 当成“更新提示”，没有把 topic 定义成权威读模型。

只要前端仍然需要：

- 从 `records` 推导其它面板，
- 在 `open` 或 timer 时再打 HTTP 校准，
- 为每个页面保留独立 fallback，

那么订阅层就永远无法真正纯化。

## Resolution

- 把主应用常驻订阅统一收口到单 `/events`，请求显式声明 `topics + resume`。
- 把每个 topic 定义成后端直接产出的权威读模型；前端只消费该 topic 的 `snapshot/replay/live`。
- 首屏 hydration 只等 topic `snapshot` 或可恢复的 `replay`，不再先发 HTTP bootstrap。
- 恢复规则固定为：
  - `schemaEpoch` 一致且 cursor 仍在 replay 窗口内时 replay
  - 否则直接发送新 snapshot
- replay 窗口用有界内存实现即可；进程重启后直接以新 snapshot 恢复，不额外补 HTTP。
- 闭合历史窗口、历史分页、非订阅页面继续走现有 HTTP，不必为了“纯 SSE”强行实时化。

## Guardrails / Reuse Notes

- 不要把 `records` 事件继续暴露成“页面自己决定要不要重拉”的契约；主应用订阅面应该直接消费 topic payload。
- 不要为覆盖范围内页面保留健康态 timer reconcile、open-resync 或页面私有 fallback；那会重新引入第二真相源。
- 不要把 closed-range / history-only 页面硬塞进持续推送；纯 SSE 的边界是“常驻当前态订阅”，不是“所有页面都实时化”。
- 如果某个 topic 仍需要短 TTL 的服务端 DB 基础快照缓存，cache key 只能包含稳定请求参数与明确允许的 bucket 维度；live runtime 状态、最新持久化行 ID 这类高抖动因子必须留在响应阶段 overlay，否则 singleflight 会被打散，订阅与 HTTP 都会继续重复重算同一份基底。
- 与 Dashboard 同屏但不共享同一 owner-facing contract 的接口，不要为了“省实现”直接复用 dashboard full snapshot builder；应复用更低层的账户活动聚合块，避免把 summary/model-performance/reconcile 之类 dashboard-only 组装再次带回实时主链路。
- 不要为 replay 失败发明第三条恢复路径。恢复规则只应是 replay 或 snapshot。
- 手动“立即重连”不应偷偷复用旧 `resume` 去赌 replay 命中。若产品语义是“人工要求重新拉一份当前态”，前端就应该对 active topics 强制 fresh snapshot，并给这次连接分配独立 `attempt/reason` 供前后端对账。
- topic 参数必须 canonicalize；否则 resume cursor 与 cache key 会漂移。
- SSE envelope 字段名也必须在端到端 drill 中被校验。若后端真实发出的字段名与前端 registry 读取约定不一致，即便 topic 设计本身是纯推送，页面仍会静默丢弃 snapshot，看起来像“连接正常但数据不动”。
- 主应用 shell 也属于订阅覆盖面的一部分。像版本信息这类看似外围的小数据，只要已声明为 `app.version` topic，就不应再额外保留 `/api/version` 首屏 bootstrap，否则网络面上仍然是混合推拉。
- owner-facing 离线提示不能只说“断线了”。至少要暴露最近连接 `attempt`、触发 `reason`、active/resume/forced-snapshot topic 数量、最近消息时间与最近终态；否则“刷新能恢复但按钮不能”的问题在现场没有可判责证据。

## References

- `docs/specs/5932d-sse-proxy-live-sync/SPEC.md`
- `docs/specs/z6ysw-dashboard-account-activity-tabs/SPEC.md`
- `src/api/slices/subscriptions.rs`
- `web/src/lib/sse.ts`
