# 代理热路径并发稳定性与传输背压收口（#q8h3n）

## 背景 / 问题陈述

- 已落地的 raw 异步旁路、records 查询收敛与 summary/quota debounce 确实缓解了 `180s first-chunk timeout`、`database is locked` 与热路径抖动。
- 但新增的 `/v1/*` whole-proxy admission gate 把本地默认并行上限锁到了 `12`，并在高峰窗口内稳定产生日志 `proxy request concurrency wait timed out` 与本地 `503 proxy concurrency limit reached; retry later`。
- 代理收尾的 `proxy capture follow-up` 曾在无 SSE 订阅者时仍先跑 `refresh_hourly_rollups_for_read_surfaces_best_effort(...)`，把 SQLite 写压和锁竞争放大到无用户可见收益的程度。
- 产品硬性要求是“至少 100 个并行调用在工作”，因此该 gate 不是保护，而是新的功能性缺陷；必须回退。

## 目标 / 非目标

### Goals

- 移除 `/v1/*` whole-proxy admission gate，确保至少 100 个同时在途请求不会被本地入口限流直接拒绝。
- 让 request raw 与 response raw 从业务转发热路径旁路出去；资源紧张时优先丢 raw，不阻塞代理流。
- 让大 body sticky/rewrite 不再默认整包内存物化；大体积 body 只做前缀探测，必要时回落到 file-backed replay。
- 让 `/api/invocations` 分页主查询只对当前页记录执行重型投影，并让 summary/quota follow-up 在 burst 写入时自动合并。
- 让 `proxy capture follow-up` 只在 active SSE subscribers 或 shutdown tail flush 时消耗 summary/quota 预算；无订阅者时不得再触发重型 rollup refresh。
- 让号池路由在近期上游传输超时后降低同一上游端点与同一代理节点组合的选择优先级，避免短窗口内连续命中同一个坏传输组合。
- 在共享测试机 `codex-testbox` 上完成 100 并行压测，验证本地 admission reject 已消失，且已有效的 hot-path 修复没有回退。

### Non-goals

- 不改变外部 API 语义、响应格式或 SSE 事件顺序。
- 不做数据库引擎迁移，不引入新的持久化后端。
- 不把问题收口为单纯的配置调参或运营规避。

## 范围（Scope）

### In scope

- `src/proxy.rs` 中 `/v1/*` 请求入口、pool 路由 body 处理、request/response raw capture 与 summary follow-up 调度。
- `src/config.rs` / `src/app_state.rs` / `src/runtime.rs` 中 whole-proxy admission gate 的移除、纯观测型 in-flight 指标与 deprecated 配置告警。
- `src/api/mod.rs` 中 invocation 列表分页查询的轻量页 id 预选 + 当前页重型投影（保留并复验，不回退）。
- `src/tests/mod.rs` 中与 100 并行、不再本地 admission reject、raw 异步和长流 in-flight tracking 相关的回归测试。
- `scripts/shared-testbox-proxy-parallel-smoke` 共享测试机压测脚本与对应的 mock upstream / loadgen harness。

### Out of scope

- retention / archive 离线链路改造。
- 详情页 raw 文件读取语义调整。
- 前端 UI 结构改造。

## 设计约束

- `/v1/*` 不允许再存在任何整机级 whole-proxy admission gate；观测可以保留，准入拒绝不允许保留。
- response raw 必须“先转发 chunk，再异步落盘”；request raw 必须“先发上游/继续流程，再异步收尾写盘”。
- raw 异步旁路必须有明确截断原因，至少覆盖 `max_bytes_exceeded`、`write_failed:*`、`async_backpressure_dropped`。
- 大 body sticky 探测只允许读取固定前缀窗口；超过窗口仍未识别时，直接回落到“无 body sticky 优化”的 replay 路径。
- summary/quota follow-up 必须具备 burst coalesce，避免每条新记录都立即跑完整汇总。
- `proxy capture follow-up` 必须先看 `receiver_count()`，无 SSE 订阅者且非 shutdown flush 时直接跳过，不消耗 follow-up seq，也不触发 summary/quota 查询或 rollup refresh。
- `PROXY_REQUEST_CONCURRENCY_LIMIT` / `PROXY_REQUEST_CONCURRENCY_WAIT_TIMEOUT_MS` 只能作为弃用兼容项继续被读取与告警，不得再影响 `/v1/*` 准入。
- `PROXY_REQUEST_CONCURRENCY_*` 不得再参与 raw writer sizing、部署卡 owner-facing 并发控制语义或新请求失败分类；历史 `proxy_concurrency_limit` failure kind 仅可用于旧记录统计兼容。
- 对 tracked `/v1/*` POST，请求分配 `invokeId + occurredAt` 后必须立即进入 runtime store 的 `running` 可见态；该可见性不得等待 body read、route context、账号选择、upstream attempt 或 SQLite record enqueue。
- 号池路由只能把近期 `transport_failure` 且 failure kind 属于 `upstream_handshake_timeout`、`failed_contact_upstream` 或 `upstream_stream_error` 的 `upstream_route_key + proxy_binding_key_snapshot` 组合纳入短期降权；同组合后续成功应清除该短期惩罚，认证、配额、402 等账号级硬失败不得混入组合降权。
- capture 入口不得为了提速跳过完整 raw、usage、failure、prompt-cache/encrypted owner 与 body rewrite 语义。可证明安全前，capture 请求必须先使用 replay snapshot 控制面读取：小体积保留内存，大体积落 file-backed replay；日志必须给出 `body_read_done`、`body_size_bucket`、`request_body_snapshot_kind` 与 `live_first_reason`，说明为何未启用 live-first。
- capture response streaming 必须先向下游转发 chunk，再异步收敛 raw/record；日志应能区分 `downstream_first_byte_elapsed` 与 `raw_response_write_elapsed`，避免把原始响应落盘耗时误判为上游首字节慢。
- 任何从完整 `Bytes` / `Vec<u8>` 构造 pool failover replay snapshot 的路径都必须经过统一阈值 helper：`<=1MiB` 才允许 `memory`，超过阈值必须优先写 `cvm-pool-replay-*` 临时文件并返回 `file` snapshot。只有临时文件创建、写入或 flush 失败时才允许 fail-soft 回退 `memory`，且必须产生日志证据。
- `prepare_pool_request_body_for_account` 在 rewrite required 但实际 no-op 时必须保留原 snapshot kind；不得因为读取 JSON 判断而把原 file-backed snapshot 重新包装成 memory。真实 rewrite 后的新 body 也必须重新经过同一阈值 helper。
- 生产默认 `RUST_LOG=info` 下必须能看到关键慢段阈值事件：body `>=1MiB` 或 read `>=1000ms` 的 `body_read_done/live_first_reason/request_body_snapshot_kind`，downstream first byte `>=2000ms`，raw response write `>=500ms` 或 raw bytes `>=1MiB`。普通小请求可继续只输出 debug，避免刷屏。

## 验收标准（Acceptance Criteria）

- `codex-testbox` 上 100 个同时发起的 `/v1/*` 代理请求不会出现任何本地 `503 proxy concurrency limit reached; retry later`。
- response raw append 不再位于 chunk 转发之前；request raw 写盘不再阻塞上游发送。
- 大于小体积阈值的 pool request body 不再默认整包内存物化；sticky 探测仅依赖前缀窗口或 replay snapshot 前缀。
- capture 大 body 读取会产生 file-backed replay snapshot；超限/超时/客户端断开时仅保留有界 partial body 证据，不得因为切换 snapshot 控制面而丢 raw failure context，也不得把成功读取的大 body 同时整包留在内存。
- capture pool outbound 与 route-selection prebuffer fallback 的大 body failover snapshot kind 必须为 `file`；11MB/21MB/62MB 等请求不得在正常临时文件可用时继续以 `snapshot_kind="memory"` 进入上游 timeout / failover 日志。
- capture 在现有完整语义仍要求 JSON parse/rewrite 时，只允许对 file-backed snapshot 做一次最终物化；不得再额外制造 `Bytes -> Vec` 级别的整包副本，也不得把该保守路径宣称为零拷贝 live-first。
- capture 日志能解释 live-first eligibility：不能证明安全的请求必须显式记录 fallback reason；后续若启用 live-first，必须覆盖 encrypted owner、prompt-cache binding、body rewrite、failover replay 与 raw 完整性测试。
- rewrite no-op 与 rewrite changed 场景都必须保持 raw request/response、terminal record metadata、usage、failure kind、prompt-cache/encrypted owner 语义不变；本 spec 不允许用截断 raw 或跳过 failover replay 换取速度。
- `/api/invocations` 的分页主查询只先选出当前页 id，再对当前页记录执行完整投影。
- summary/quota follow-up 在 burst 写入时能够合并，不再对每条记录立即触发一次完整汇总。
- 无 SSE 订阅者时，`persist_and_broadcast_proxy_capture` 与 `broadcast_recovered_proxy_invocations` 不会再启动 follow-up worker，也不会触发后台 hourly rollup refresh。
- active subscriber 场景仍保持 `records -> summary/quota` 近实时广播；shutdown tail flush 语义不回退。
- 即使线上环境仍设置 `PROXY_REQUEST_CONCURRENCY_LIMIT` / `PROXY_REQUEST_CONCURRENCY_WAIT_TIMEOUT_MS`，它们也只会产生日志告警，不会改变 `/v1/*` 准入行为。
- raw writer 并发上限使用独立默认值/配置，不得从 deprecated `PROXY_REQUEST_CONCURRENCY_*` 推导。
- started/admitted/running-shell 观测应能证明本地 admission reject 为 0，且 `max_proxy_in_flight_observed` 可以超过旧配置值。
- Given 某 `upstream_route_key + proxy_binding_key_snapshot` 近期发生超时/传输失败，When 号池还有其它可路由组合，Then 路由排序优先尝试其它组合；若只有该组合可用，仍允许回退使用而不是直接报无账号。

## 参考

- `/Users/ivan/.codex/worktrees/4032/codex-vibe-monitor/src/proxy.rs`
- `/Users/ivan/.codex/worktrees/4032/codex-vibe-monitor/src/api/mod.rs`
- `/Users/ivan/.codex/worktrees/4032/codex-vibe-monitor/docs/specs/n7c2r-proxy-hot-path-no-raw-reread/SPEC.md`
