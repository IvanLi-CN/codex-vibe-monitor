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
  - docs/specs/pbgwc-prompt-cache-conversation-bindings/SPEC.md
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
- 详情抽屉可以采用混合边界：仅用 scope-specific topic 推送最新可见窗口，历史深分页固定在首次 HTTP `snapshotId`。合并时按稳定键替换当前行，只有新的稳定键才算“新数据”；离开列表顶部时延迟插入，避免破坏阅读锚点。

## Guardrails / Reuse Notes

- 不要把 `records` 事件继续暴露成“页面自己决定要不要重拉”的契约；主应用订阅面应该直接消费 topic payload。
- 不要为覆盖范围内页面保留健康态 timer reconcile、open-resync 或页面私有 fallback；那会重新引入第二真相源。
- 不要把 closed-range / history-only 页面硬塞进持续推送；纯 SSE 的边界是“常驻当前态订阅”，不是“所有页面都实时化”。
- 对 summary 这类同时存在 open-range 与 closed-range 的读面，也要明确 owner-facing 边界：`stats.summary.current` 适合 `today / 1d / 7d` 这类 open-range 当前态，不适合 `previous7d` / `yesterday` 这种 closed-range exact window。closed-range 若仍挂在 topic 上，只会把 archive repair/fallback 压力钉在高频订阅链路里。
- 如果某个 topic 仍需要短 TTL 的服务端 DB 基础快照缓存，cache key 只能包含稳定请求参数与明确允许的自然日锚点等低抖动维度；移动中的 exact `rangeStart/rangeEnd`、live runtime 状态、最新持久化行 ID 这类高抖动因子必须留在响应阶段 overlay，否则 singleflight 会被打散，订阅与 HTTP 都会继续重复重算同一份基底。对应的 invalidate 粒度也必须与同一 selection 对齐；单个 terminal 事件只能清掉匹配 selection，不能把整张 dashboard snapshot cache 一起 flush。
- 对 `dashboard.activity.current` 这类 open-range topic，`live` 广播不应该再反向触发完整 DB snapshot builder。终态在 accepted/enqueued 后应同步以幂等 delta 写入共享内存累计 baseline；固定 5 秒 deadline 只负责合并发布内存 snapshot。DB baseline 以更长、明确的 reconcile cadence（例如 60 秒）刷新，reconcile 失败保留 last-good totals 与 live overlay，而不是重新把 terminal burst 变成 cache invalidation 风暴。
- rolling duration 窗口若没有完整的到期反向 delta，不能只重写响应的 `rangeStart/rangeEnd` 后复用较旧 baseline；这会把窗口外记录伪装成当前 totals。此时必须把该窗口的缓存上限收紧到其允许的可见时效预算，或先实现可验证的 expiry delta，再延长 DB reconcile cadence。
- expiry delta 本身也是内存队列：baseline boundary load、in-flight replay 和后续 live terminal 必须共用排序与容量约束。expiry horizon 应在 boundary query 前解析并随 entry 保存，命中不得越过已捕获上界；live-only expiry 无法覆盖 archive 前缀时应明确 fallback，不能缓存不完整的反向增量。
- Dashboard open-range 的 5 秒 publish 与 60 秒 reconcile 必须由两个独立 deadline 驱动。terminal burst 只能合并内存发布；成功、并发写入后重放、last-good 和失败都要推进 reconcile attempt deadline，避免错误路径绕过节流。
- 当 SQLite writer 已处于 busy/locked cooldown 时，60 秒 reconcile 不是提高优先级的理由。已有 baseline 的 open-range selection 应返回带 live overlay 的 last-good snapshot，并将 `reconcile_outcome=deferred`、`reconcile_skip_reason=writer_pressure`、baseline age 和下一次 due 时间写入 telemetry；超出明确的最大延后窗口后才尝试补偿构建。
- terminal persistence journal 只负责有限窗口的 admission/replay，不取代累计 read model。5 秒 topic publish 必须继续只读内存 snapshot，不能因为 journal pending 就触发同步 DB builder；P1 SQLite ACK 与 P2 derived work 的延迟应分别判责。
- coverage repair 应优先 owner 正在使用的闭合小时，并受 bucket 数与 elapsed 双预算约束；历史全量 backlog 连续无进展时应指数退避，永久 payload-required blocked target 不得被计入 actionable backlog 或触发高频 hourly refresh。
- `response_source=memory` 只能描述实际请求没有执行 DB build 的结果。若本次先构建 DB baseline 再返回 last-good，必须同时记录 `build_attempted=true`、`build_source` 与 reconcile outcome，不能用最终 payload 来源掩盖本次数据库成本。
- topic 刷新必须使用连接级 owner subscriber 引用计数，而不是内部 broadcast receiver 数量；没有 owner subscriber 时只标记 dirty。重新订阅时应清除旧 replay 连续性并构建 fresh snapshot，避免把失活期间积累的旧 ring 当作权威连续状态回放。
- Summary open-range 的 terminal refresh 应使用固定 `500ms` deadline；同一 deadline 内后续事件只累计 `coalesced_event_count`，不延长窗口。刷新失败保留 last-good totals，并记录 `refresh_outcome`、`last_good_age_ms` 与有界 retry backoff。
- proxy terminal follow-up 如果没有真实 quota owner subscriber，应直接跳过 quota refresh；不要用 broadcaster receiver 数量作为 owner-facing 订阅判断，也不要继续构建无生产消费者的 legacy Summary 窗口。
- Dashboard / upstream-account 的 recent 预览不得再为了补当前态而对整个选中 range 扫 persisted `running/pending` 行；当前态应来自 runtime/live read model，旧持久化运行态最多只能作为 bounded recent 候选参与展示。
- `stats.summary.current` 与 `/api/stats/summary` 的 open-range `usage_breakdown / non_success_tokens` 也不能再借道 raw preview rows 或 live invocation id overlap scan。若 summary 仍需要 `7d` / `today` / 长 duration 的模型分组或非成功 token，优先复用 live aggregate + archive aggregate merge；只有在 materialized archive 无法提供所需明细时，才允许显式 fallback/置空，而不是悄悄扫整窗 raw rows。
- 如果 Dashboard / upstream-account 的 `usage_breakdown` 还需要 `model + reasoning` 维度，则应和 summary 一样共用单一的 `rollup + exact tail` builder，并把 fallback 限定在“缺 replay marker 的 archive hole”这类显式不健康窗口。不要让 dashboard full、upstream-account、summary 三个入口各自保留一份 7d raw aggregate 逻辑，否则单个页面 bundle 仍会通过不同 route 反复打 SQLite。
- Archive-hole fallback 的健康目标不是隐藏告警，而是让可修复缺口尽快消失。对 pruned legacy archive，breakdown rollup 可结构化 replay 并用空/unknown reasoning 兜住不可恢复维度；只有 archive 不可读、repair 仍有部分进度，或真正 payload-required 的 target blocked 时，fallback/blocked telemetry 才应该继续出现。
- 如果 read path 对某个 rollout 中的新 rollup target 仍有依赖，就要把这类 backlog 的修复调度前置到 startup / bounded backfill 的高优先级 pass，并和永久 blocked target 分开统计。否则即使 owner-facing 已不再需要 topic 订阅，该 target 也会在每次冷加载/切窗时继续走同一条 archive fallback。
- Dashboard 账号累计活动不应每个 5 秒 refresh 都重跑整窗 conversation/window SQL。稳定做法是让 Dashboard full 与 upstream-account 共用逐小时 coverage planner：完整且已覆盖的小时读版本化 rollup，hole 只回退对应连续小时，leading/trailing partial hour 保持 exact；latest timestamp/value 必须作为一对合并，conversation created-at 可由 keyed hourly rollup 补齐。
- `wait_on_in_flight=0` 本身不能证明 cache key 抖动；没有并发同参请求时它是正常结果。判责日志应同时记录 `refresh_reason`、`invalidation_reason`、稳定 selection fingerprint、base snapshot age 与 TTL，才能区分正常 TTL rebuild、显式 topic invalidation 和真正的 selection 漂移。
- 与 Dashboard 同屏但不共享同一 owner-facing contract 的接口，不要为了“省实现”直接复用 dashboard full snapshot builder；应复用更低层的账户活动聚合块，避免把 summary/model-performance/reconcile 之类 dashboard-only 组装再次带回实时主链路。
- 不要为 replay 失败发明第三条恢复路径。恢复规则只应是 replay 或 snapshot。
- 手动“立即重连”不应偷偷复用旧 `resume` 去赌 replay 命中。若产品语义是“人工要求重新拉一份当前态”，前端就应该对 active topics 强制 fresh snapshot，并给这次连接分配独立 `attempt/reason` 供前后端对账。
- topic 参数必须 canonicalize；否则 resume cursor 与 cache key 会漂移。
- SSE envelope 字段名也必须在端到端 drill 中被校验。若后端真实发出的字段名与前端 registry 读取约定不一致，即便 topic 设计本身是纯推送，页面仍会静默丢弃 snapshot，看起来像“连接正常但数据不动”。
- 主应用 shell 也属于订阅覆盖面的一部分。像版本信息这类看似外围的小数据，只要已声明为 `app.version` topic，就不应再额外保留 `/api/version` 首屏 bootstrap，否则网络面上仍然是混合推拉。
- owner-facing 离线提示不能只说“断线了”。至少要暴露最近连接 `attempt`、触发 `reason`、active/resume/forced-snapshot topic 数量、最近消息时间与最近终态；否则“刷新能恢复但按钮不能”的问题在现场没有可判责证据。
- 同一详情抽屉的不同区域应拆成独立 topic，并且仅订阅当前可见 tab：Records 只刷新匹配 scope 的调用/概览，提交后的配置变更只刷新绑定/事件。重型概览可以有固定短合并窗口；所有 topic 刷新失败都保留 last-good payload，恢复仍只走 replay 或 fresh snapshot。

## References

- `docs/specs/5932d-sse-proxy-live-sync/SPEC.md`
- `docs/specs/z6ysw-dashboard-account-activity-tabs/SPEC.md`
- `src/api/slices/subscriptions.rs`
- `web/src/lib/sse.ts`

## Shared Projection Consumers

- Dashboard 和长期统计可以共享 terminal admission/ACK Hub，但 5 秒 Dashboard publish 与 60 秒长期物化必须是独立 deadline；后者永远不能反向触发 Dashboard DB build。
- `response_source=memory` 之外还应记录 projection trigger、cursor lag、dirty bucket、flush outcome 与 pressure defer，避免把后台物化成本误标为纯内存命中。
- `stats.timeseries.open-window` is another projection consumer: full minutes read durable aggregates, unflushed terminal deltas are overlaid from the Hub, and only boundary minutes may use exact raw reads. Its 60-second P2 flush must not share the Dashboard publish deadline or force a topic rebuild.

## Memory Attribution Guardrail

- Dashboard、terminal projection 和长期统计的内存问题必须先分类再修复。5 秒/60 秒 cadence 不得被观测采样改变；采样应读取现有容器的容量、长度和 proc/cgroup 指标，不能克隆 snapshot 或为诊断发起 raw/SQLite 扫描。
- `managed_bytes` 只能表示已知缓存的保守估算；匿名 RSS 中剩余部分记录为 `unattributed_anon_bytes`，用来区分 allocator、SQLite page cache 和尚未覆盖的组件。不得把数据库行数直接映射成 runtime memory。
- 只有在连续生产采样证明某一组件达到 RSS 的主要占用阈值，或一次操作造成持久的 `VmHWM` 峰值，才单独设计分块、LRU 或临时对象生命周期修复；第一阶段不设置硬 RSS 上限、不丢事件、不降低并发。

## Runtime Projection Boundary

- Dashboard live, terminal totals, and runtime overlays are separate projections with explicit cadences: current-state updates may coalesce at 250ms, terminal totals at 5s, and SQLite baseline reconcile at 60s. A terminal burst must never turn the 5s publish deadline into a database rebuild loop.
- SSE producers serialize one immutable frame per revision and fan it out by shared byte chunks. Subscriber count must not multiply builder or JSON serialization work; an unchanged revision must not advance the topic cursor.
- “每个 revision 只序列化一次”必须从 producer 到 subscriber 全链成立。若 producer 仍广播完整业务 snapshot，再由多个 topic 克隆 cached payload、修改 `serde_json::Value` 并分别序列化，那么共享 frame 只消除了同 topic 的 subscriber 放大，没有消除单页面的多 topic 放大。
- 高频 projection 应按数据变化性质拆分 revision 和 cadence：current/phase 可使用 `250ms`，network/rate 使用 `1s`，terminal totals 使用 `5s`。网络可见性不能为了保持活跃而重新标记完整 Dashboard projection。
- 性能验收必须驱动真实 active-subscriber producer、全部页面 topic 与 mutation fan-out。单独证明零 SQL、单 topic Arc 复用或合成 snapshot p95，不能替代 Dashboard tab on/off CPU A/B。
- `response_source=memory` is valid only when the request did not execute a database build. Reconcile attempts must additionally record build source, outcome, baseline age, active subscriber count, and pressure deferral so memory delivery cannot hide database cost.
