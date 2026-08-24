# High-Frequency Runtime Data Plane

> 本文定义代理请求语义、运行态投影、Dashboard 实时快照、SSE 分发与持久化对账之间的跨域边界。实现状态见 `./IMPLEMENTATION.md`，关键决策见 `./HISTORY.md`。

## 背景

项目已经具备 terminal journal、Terminal Projection、Dashboard read model、SQLite rollup 与 SSE topic，但这些能力没有形成一条可执行的统一边界：Dashboard live 构建仍可反向读取 SQLite，订阅分发按消费者重复克隆和序列化 payload，file-backed 大请求会在 dispatch 阶段重新整体物化，writer queue 的跨阶段 accounting 也可能下溢并污染内存归因。

该问题不能继续通过逐条慢 SQL 或延长 TTL 收口。高频数据面必须明确区分 ingress、projection、delivery、persistence/reconcile 五层，并在类型和测试层禁止反向依赖。

## Goals

- 建立 `RuntimeProjectionHub`，统一接收 runtime upsert/remove、phase、network、account metadata 与 terminal delta，并为 Dashboard 当前态提供健康内存快照。
- 每个代理请求只生成一份 replay snapshot 与一份 `RequestSemanticProjection`；超过 `1 MiB` 的 body 始终 file-backed，并保持现有 `256 MiB` 上限、路由、重写、failover 与 raw capture 语义。
- Dashboard 当前态以固定 `250ms` deadline 合并；terminal totals 保持 `5s` 可见；SQLite baseline 仅用于启动、`60s` reconcile、明确冷回退和 closed-range exact 查询。
- 每个 topic revision 只序列化一次，并以共享不可变 frame 进入 cache、replay ring、broadcaster 与 subscriber。
- 修复 writer accounting 的所有权和不变量，使运行健康与内存归因可被验证；生产镜像默认 `MALLOC_ARENA_MAX=8`，同时保留部署环境覆盖能力。
- `GET /api/system/status` additive 暴露 `runtimePressureHealth`，不改变 Dashboard、统计、raw detail 或 SSE 的既有 wire shape。
- `GET /api/stats/summary` 由事件驱动的内存 projection 提供，启动 hydration 后后台刷新最长间隔为 15 秒；HTTP 请求（包括 cache miss 或 TTL 到期）不得执行 SQLite 或启动 snapshot rebuild。刷新失败仅可在 exact last-good 未超过 15 秒时保留既有响应形状；超过边界使用既有 unavailable 错误契约，不得返回过期或零值数据。

### Summary Projection

`GET /api/stats/summary` 的真值是 `SummaryProjection`，不是按 URL 参数保存的一组 SQL 结果。readiness 前，projection 从 invocation、hourly rollup、archive、账号 usage、recent invocation、maintenance 与 runtime overlay 的持久化基线完成 hydration；readiness 后 HTTP 只解析和规范化参数，并从 projection 精确派生原有 `StatsResponse`。公开 rolling duration 合同上限为 30 天（`60d`、`12mo` 等超限输入在访问 projection/SQLite 前返回既有 400）。48 小时以内保留任意分钟/小时精度；超过 48 小时的 rolling duration 必须使用整日粒度，以保证部分小时边界有限且可在后台预取；不满足粒度的输入返回既有 400，绝不隐式截断。任何受支持的 `window`、`timeZone`、`upstreamAccountId` 与 `limit` 组合不得借用另一组合的结果，也不得在 HTTP 内查询 SQLite、检查路径或扫描文件。

projection 按账号（含全局合并）、UTC 时间桶和 recent invocation 顺序保留以下可组合输入：成功/失败/非成功计数与成本、token 和 usage/model/reasoning 细目、延迟样本与直方图、archive/rollup coverage、活动 runtime phase/等待计数、terminal overlay，以及 maintenance 的 last-good 快照。calendar 和 previous-day 选择在请求内由 canonical timezone 将内存 UTC bucket 切为精确区间；rolling window、all-time 和 current-limit 分别从 bucket、累计聚合和 bounded recent index 派生。recent index 的后台输入按 occurred-at 有界读取；当溢出输入不能证明某账号的 requested-N 完整时，该选择采用既有 unavailable 契约，绝不返回缩短或零值结果。raw fallback 的 exact bucket 集限制为 4096 个；超限时后台 hydration 不发布部分 projection，而保留 exact last-good 或采用 unavailable 契约。48 条限制仅约束可选的已序列化 response LRU，不能限制 projection 对可证明选择的精确性。

持久写入完成和 typed runtime mutation 都发出合并触发信号；后台维护器以单飞锁、250ms 内存调度相位、mutation 专用 250ms debounce、10 秒最小重建间隔和 4 秒 runtime deadline 重读持久化基线。调度相位、hub 协调余量与 runtime build deadline 必须严格落在 15 秒 freshness 预算内；readiness 前的首份完整 hydration 独立执行，服务在其成功前保持不可用而非把有效但较慢的启动 build 当作 runtime timeout。live exact tail 保留 48 小时，历史归档只保留已支持 rolling/calendar 窗口所需的边界小时，完整小时由有界 rollup 与 account coverage 派生。每个成功 projection revision 记录单调时钟；HTTP 只返回精确 revision 年龄不超过 15 秒的 response。`all` 聚合拥有独立的成功刷新时钟，普通窗口刷新不能延长它的年龄。刷新失败时，同一选择的 last-good 只在该 15 秒边界内可用；超过边界，端点采用既有 unavailable 契约，绝不返回过期、空值或跨选择数据。System Status 的 durable refresh 在每一轮 60 秒 TTL 前启动，并以同一周期和 4 秒 deadline 保持 last-good 服务窗口。

## Architecture

### Ingress

- 请求入口拥有唯一 `PoolReplayBodySnapshot`，并在单次 visitor pass 中产出不可变 `RequestSemanticProjection`。
- projection 至少包含 model、stream、sticky/prompt-cache key、reasoning、service tier、encrypted/image/compaction 与是否需要 `include_usage` rewrite。
- 小请求可驻留内存；超过 `1 MiB` 的请求必须保留 file-backed snapshot。需要 rewrite 时使用有界流式转换，业务缓冲不得超过 `64 KiB`。
- 语义转换失败保持当前 fail-open 原始 body 行为，并记录明确原因；不得因优化改变转发字节或路由结果。

### `/v1/responses` live request body

- `/v1/responses` 只可在首轮最终路由已经冻结后向上游发送首个请求体字节；原始下游字节同时持续写入同一份 replay snapshot，failover 从该 snapshot 重建请求体。不得先发送再因晚到路由字段取消该次上游请求。
- 请求级路由快照在请求开始时冻结。model、有效 sticky/prompt-cache 绑定、已启用的加密会话 owner 以及实际存在图片能力差异时的图片意图都必须确定；`tools` 与 `tool_choice` 只在后者为真时构成依赖。根对象字段顺序和重复键不构成客户端限制，无法在有界解析预算内安全最终化时走既有完整缓冲路径。
- 高基数 prompt-cache/encrypted-owner 路由键使用容量 16,384 的 LRU，缓存空结果；冷 miss 以单飞加载，绑定写入精确失效对应键。低基数设置、模型映射、压缩、超时与实验开关作为版本化 runtime snapshot 发布。热请求不得有固定 SQLite 读取；snapshot 或键缓存不可用时保守缓冲。
- 逻辑 JSON 的增量变换覆盖 `stream_options.include_usage` 和 OAuth `/v1/responses` 的既有 rewrite 规则。输出以 JSON 语义等价为边界，不承诺字段顺序、空白或压缩字节相同；已经发送前缀后发现非法 JSON 时取消上游并向下游返回现有 `400`。
- live-first 支持现有 `follow`、`identity`、`gzip`、`deflate` 与 `zstd` 请求压缩策略。变换或重新编码的 body 不携带旧 `Content-Length`；不支持的 inbound encoding 沿用现有拒绝行为。
- 设置默认关闭。启用后使用 `hash(invoke_id + live_first_revision) % 100` 固定按 treatment 百分比分配 control/treatment；control 保持完整缓冲，跨账号重试不改变 variant。`responses-live-request-body-v2` 与 v1 故障样本隔离；功能关闭时不运行新的路由分析或写入新指标。现有 failover 语义不因该实验改变，但仅实际开始消费上游 body 后的失败才记为不确定交付。

### Projection

- `RuntimeProjectionHub` 是 current-state 的唯一高频事实层，接收 runtime 与 terminal 事件并维护 Dashboard 所需的全局及账号投影。内部必须按 current/phase、network/rate 与 terminal totals 拆成独立不可变切片和 revision，分别使用 `250ms`、`1s` 与 `5s` 固定 deadline。
- `network_visibility` 只能推进 network 切片，不能重新标记或构建完整 Dashboard activity/summary projection。未变化切片不得推进 revision。
- `DashboardLiveProjection::snapshot()` 不接受 `Pool<Sqlite>`、数据库 repository 或可执行 SQL 的闭包。健康 live render 的数据库查询数必须为零。
- terminal durable 事实继续由 `TerminalProjectionHub` 与 P1 journal 管理；两个 Hub 共享 ingress 事件标识，不共享可变 ownership 或回收 cursor。
- startup warm restore、`60s` reconcile 与 cold fallback 可访问 persistence；已有 last-good 时，订阅请求链不得同步回源数据库。

### Delivery

- `TopicMaterializer` 只接受 typed base 与其依赖切片的 revision tuple，并生成一个 `Arc<SerializedTopicFrame>`。frame 包含 envelope bytes、cursor、schema epoch、fingerprint 与 topic metadata。
- Dashboard activity 与 summary live overlay 的 typed base 必须按 current/network/terminal dependency revision 原地 materialize；network timeseries 与 network recent 也必须使用 typed serializer。生产高频路径不得广播完整 `DashboardActivityLiveSnapshot`、深拷贝 cached topic 或修改通用 `serde_json::Value`。
- Activity 的 terminal typed base 必须保留模型性能与账号延迟聚合输入，并以有界 recent projection 更新已有的 recent 语义；它不得为此重新读取 SQLite 或广播完整调用记录。
- 当 SQLite baseline 已包含 queued terminal 时，Activity typed base 必须继承该 terminal sequence；同一 shared terminal slice 不能在 cold base 上再次累计。
- 活动日历窗口或 activity/summary rolling-duration 窗口的 typed base 到达其 range anchor 边界时，必须由 producer-owned runtime reconcile 在 revision delivery 之外受控重建；duration 复用既有 `60s` reconcile 边界，且 terminal materialization 推进的公开 range 输出不得重置 base 的 rebase anchor。terminal materialization 只隔离陈旧 base，不得读取 SQLite；无 owner 时陈旧 base 保持 dirty 并在 ownership 返回后重建，重建失败时隔离持续到下一次 reconcile，且 byte-identical 重建不得推进 frame cursor。
- cache、replay ring、broadcaster 和 subscriber 只共享 frame 引用；不得接收 `serde_json::Value` 后再次序列化或深拷贝 payload。
- 首个 owner subscriber 激活 producer；后续 subscriber 只增加引用计数。无 owner subscriber 时停止周期 producer，mutation 只标记 dirty。
- projection revision 未变化时不推进 cursor，不发送重复 frame。

Activity、summary 与 network topic 已建立上述 typed delivery 基础；working-conversations、parallel-work open range 与 open-window timeseries 的剩余强制迁移由 [`dashboard-hot-topic-projection`](../dashboard-hot-topic-projection/SPEC.md) 规范。跨域数据面不能仅凭前一组 topic 的完成状态宣称整个 Dashboard 高频路径已经收口。

### Persistence And Reconcile

代理请求热写必须经过统一写协调器。P1 terminal、同步 attempt/route 与 P2 derived 的优先级固定，禁止同一代理生命周期内的多个 helper 独立争抢 SQLite writer。P1 采用有界短批次并在 commit 后 ACK；锁冲突保留完整批次并指数退避，新事件只能合并，不能重置 retry deadline。P2 不得在一个事务内无界追赶 rollup cursor。

- SQLite 是 terminal durable source、projection warm restore、closed-range exact query 与 drift reconcile 的事实源，不是 Dashboard current-state 的请求内查询依赖。
- terminal totals 使用 `5s` 内存发布，baseline reconcile 使用 `60s` cadence。压力或 last-good 状态机沿用既有退避与精确恢复语义。
- `PendingQueueAccounting` 统一拥有 enqueue、coalesce、batch replacement、P1 -> P2 transfer 与 completion 的 byte/depth 变化；业务阶段不得直接执行裸 `fetch_sub`。
- accounting 不变量破坏必须进入 degraded health 并保留证据，不能 wrap 到 `usize::MAX` 或继续报告 healthy。
- startup backfill 的 supervisor 按持久化 task deadline 或匹配 repair wake 调度，不使用全局固定 ticker。空闲等待不得扫描 task progress、执行无关 maintenance 或写 `system_task_runs`；source-unavailable task 只在相关 archive/payload/coverage 输入变化或每日受限 probe 时运行。

## Public Contracts

- Dashboard、统计、raw detail HTTP response 不变。
- SSE topic 名称、schema epoch、snapshot/replay/live envelope、排序、recent 与 range 语义不变。
- `GET /api/system/status` 可 additive 增加 `runtimePressureHealth`；旧前端在字段缺失时按 unknown 兼容。
- 现有 pool routing settings additive 暴露 `liveRequestStreaming: { enabled, treatmentPercent }`；默认值为 `false` 和 `50`，启用后对所有符合其他请求条件的 `/v1/responses` 生效。
- `GET /api/stats/perf` 可按 endpoint、group 与 live-first revision 过滤，并 additive 返回 `liveRequestStreaming.cohorts` 与 `routeFinalization`。cohort 使用 `buffered`、`live_first`、`unknown` 三种 transport mode；性能面板默认只比较 v2，历史缺字段只能归入 `unknown`。
- typed runtime mutation bus 是唯一的生产热路径。`DASHBOARD_RUNTIME_PROJECTION_MODE=legacy` 与 `PROMPT_CACHE_TOPIC_PROJECTION_MODE=legacy` 已被移除；遗留值不得重新启用旧的完整记录广播或 topic 全窗重建。请求语义流水线的独立运维配置不属于 runtime bus 回退面。

## Runtime Pressure Health

`runtimePressureHealth` 只读取内存计数器，至少覆盖：

- Dashboard producer 状态、active topic/subscriber 数、各投影切片 revision/cadence miss、live-path DB read count、materialize/serialize count、frame bytes、subscription lag/skipped 与 last-good age。
- request pipeline snapshot kind、semantic parse count、whole-body materialization count、rewrite buffer peak 与 fallback reason。
- RSS anonymous、Swap、managed/unattributed bytes、allocator arena 配置与 writer accounting health。
- accounting pending depth/bytes、最近 invariant violation、P1 -> P2 transfer 与 degraded reason。

状态数据不得包含 payload、调用 ID、凭据或原始 SQL，也不得为了刷新状态页新增数据库查询。

## Telemetry

- projection: `projection`, `trigger`, `revision`, `render_elapsed_ms`, `live_path_db_read_count`, `snapshot_origin`, `last_good_age_ms`。
- request pipeline: `snapshot_kind`, `body_size_bytes`, `semantic_parse_count`, `whole_body_materialization_count`, `rewrite_buffer_peak_bytes`, `fallback_reason`。
- live request body: `request_body_transport_mode`, `live_first_revision`, `live_first_experiment_variant`, eligibility/reason、raw/logical body bytes、`route_finalization_{raw,logical}_bytes`、比例、耗时、结果、依赖因素及 hot-cache/cold-load 状态、`upstream_request_first_byte_ms`、`request_body_capture_complete_ms`、`request_upstream_overlap_ms`、直接测得的 `first_response_byte_total_ms` 与 `first_token_ms`。所有值共用请求 body 消费起点；不得由互相重叠的阶段耗时相加推导首响应。
- effectiveness 仅以最终成功 invocation 为分母；首尝试失败、capture failure、retry/fallback、client abort 与 `ambiguous_upstream_delivery` 作为独立风险计数。持久化不得包含正文或凭据。
- delivery: `topic_key`, `active_subscriber_count`, `builder_count`, `serialization_count`, `frame_bytes`, `frame_reused`, `cursor_advanced`。
- accounting/memory: `pending_depth`, `pending_bytes`, `accounting_transfer_bytes`, `accounting_invariant`, `rss_anon_bytes`, `swap_bytes`, `managed_bytes`, `unattributed_anon_bytes`。
- healthy/no-change 高频事件降为 debug；DB live read、whole-body materialization、accounting invariant violation、持续 stale 与序列化重复保留 warning。

## Verification

- `16 MiB` 与 `64 MiB` file-backed 请求只进行一次语义解析，业务峰值缓冲不超过 `64 KiB`；转发、编码、failover 与 `include_usage` 结果保持一致。
- 10,000 次 runtime mutation 后健康 live render 的 SQL query count 为 `0`。
- 同 topic 从 1 个增长到 N 个 subscriber 时，builder、serialization 与完整 payload clone 次数不增长；每个 revision 只有一个 frame。
- Dashboard current-state 更新 p95 不超过 `400ms`，terminal totals 在 `5s` 内可见。
- P1 -> P2、coalesce、retry 与 retained batch 后 accounting 与真实队列估算一致，不出现下溢。
- P2 pressure defer 不是执行失败，不得增加 retry 计数。健康派生写采用固定 250ms 合并；cooldown 到期或 background eligibility generation 变化负责唤醒，P1 的 20ms admission ticker 不得轮询 P2。
- 高频 Prompt Cache topic 必须使用 active-topic scoped projection。任意 Records 广播不得触发 full-window hydrate；topic delta 500ms 合并，last-good baseline 最多每 60 秒 pressure-gated reconcile 一次。
- 生产受控 A/B 中新增 Dashboard tab 的 CPU 增量不超过 10 个百分点，subscription lag/skipped 为零；连续 12 小时 RSS p95 不超过 `2 GiB` 且 Swap 不持续增长。该 A/B 是架构完成门槛，不能由“零 SQL”或单 topic Arc 复用测试替代。
- 阻塞 `/v1/responses` 的下游尾部 body 后，treatment 不得在最终路由前向上游提供首个请求 chunk；control、metadata/tools/图片/加密字段晚到与重复 root key 都不得触发“已发送再取消”。EOF、冷缓存或解析预算退化保留完整缓冲。
- API Key 与 OAuth、`follow|identity|gzip|deflate|zstd`、重复 key、嵌套 metadata、malformed body、cancel、early upstream return 与 replay/failover 均有回归覆盖。
- 性能比较必须给出每 cohort 的成功样本数及首响应、首 token、overlap 的 p50/p90/p99；每组少于 200 个成功样本时 UI 不得宣称收益结论。

## Non-goals

- 不迁移 SQLite，不扩大连接池，不提高 slow threshold，不切换全局 allocator。
- 不降低代理并发、请求体上限、统计精度或 raw/terminal 保留。
- 不把 closed-range exact 查询和非 Dashboard 页面全部迁入 Runtime Projection。
- 不用 telemetry 标签代替类型边界、query-count、parse-count 与 A/B 证据。

## Visual Evidence

以下证据由 mock-only Storybook canvas 在真实浏览器视口生成，不依赖生产数据或登录状态。运行压力状态使用 `1660x900` 桌面与 `393x852` 移动 CSS px；请求体实时转发性能组件使用 Storybook 绑定的 `desktop1280` 视口，设置面板证据使用应用一致的 `vibe-dark` 深色主题。

PR: include

![System Status runtime pressure degraded state on desktop](./assets/runtime-pressure-desktop.png)

PR: include

![System Status runtime pressure accounting error state on mobile](./assets/runtime-pressure-mobile.png)

- source_type: storybook_canvas
  story_id_or_title: Stats/LiveRequestStreamingPerfPanel/Measured
  target_program: mock-only
  capture_scope: element
  requested_viewport: desktop1280
  viewport_strategy: storybook-viewport
  margin_policy: require_margin
  evidence_surface: component
  sensitive_exclusion: N/A
  submission_gate: owner-confirmed
  state: 两个 cohort 均达到 200 个成功样本
  evidence_note: 验证 buffered-control 与 live-first-treatment 并列呈现 P50 首响应、首 token、上传重叠、重试风险，以及首响应和首 token 的绝对/相对收益。

PR: include

![Live request streaming measured cohort comparison](./assets/live-request-streaming-perf-measured.png)

- source_type: storybook_canvas
  story_id_or_title: Stats/LiveRequestStreamingPerfPanel/InsufficientSamples
  target_program: mock-only
  capture_scope: element
  requested_viewport: desktop1280
  viewport_strategy: storybook-viewport
  margin_policy: require_margin
  evidence_surface: component
  sensitive_exclusion: N/A
  submission_gate: owner-confirmed
  state: 两个 cohort 均少于 200 个成功样本
  evidence_note: 验证样本不足时在两个 cohort 明确显示 17 / 200，三项收益固定为 -，界面不将对照数值作为可用的收益结论。

PR: include

![Live request streaming insufficient sample guard](./assets/live-request-streaming-perf-insufficient-samples.png)

PR: include

![Pool routing live request streaming enabled without account group field](./assets/pool-routing-live-streaming-no-account-group-dark.png)

- source_type: storybook_canvas
  story_id_or_title: Settings/Components/Pool Routing Settings Card/LiveRequestStreamingEnabled
  target_program: mock-only
  capture_scope: element
  requested_viewport: desktop1280
  viewport_strategy: storybook-viewport
  margin_policy: require_margin
  evidence_surface: component
  sensitive_exclusion: N/A
  submission_gate: owner-confirmed
  state: 保存后实时转发保持开启；实验组占比为 50%
  evidence_note: 深色主题内外一致，实时请求体流式转发区域没有账号组字段；保存响应归一化后保留已开启状态，并按实验组占比对满足路由条件的 `/v1/responses` 请求分配 cohort。
