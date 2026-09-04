# 代理热路径并发稳定性与传输背压收口 - Implementation

## Current State

- Canonical spec: `docs/specs/q8h3n-proxy-hot-path-streaming-stability/SPEC.md`
- Implementation summary: 已完成
- Legacy invocation raw codec inference is now an atomic one-time schema migration. Its persistent marker eliminates repeated full-table UPDATE scans on completed databases, while the older raw-blob link seed marker remains a verified compatibility completion proof.
- 最新收口：`proxy capture follow-up` 已改成 subscriber-aware，`receiver_count()==0` 且非 shutdown flush 时不会再消耗 follow-up seq 或触发 summary/quota / rollup refresh；active subscriber 与 shutdown tail flush 语义已回归验证。
- Response raw capture stores identity bytes as Zstd, retains pre-compressed wire bytes without a second compression pass, and uses a CRC-framed persistent overflow spool when the bounded writer pool is saturated. Incomplete spool files are retained for inspection; complete files replay during startup.
- Overflow spools rotate at 16 MiB and reserve a 512 MiB directory budget. If a new spool cannot be admitted, capture records `capture_unavailable:spool_capacity`; it does not create a second in-memory overflow queue or delay downstream forwarding. The paired pool invocation and attempt share one finalized response raw blob with independent persistent links.

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Note: 已移除错误的 whole-proxy admission gate，并清理 `PROXY_REQUEST_CONCURRENCY_*` 配置面；共享测试机 `codex-testbox` 100 并行压测通过，确认 `/v1/*` 不再因本地 admission gate 返回 `503`。
- Note: `PROXY_REQUEST_CONCURRENCY_*` 不再被 active code 读取、告警或用于 raw writer sizing；请求入口保留 `proxy_request_in_flight` 纯观测计数，并补充 `proxy_request_started`、`proxy_request_admitted_observed`、`max_proxy_in_flight_observed` 与 `running_shell_emitted` 证据。
- Note: tracked proxy capture 请求在 route context 解析前创建内存 running shell；route validation failure 会发 terminal overlay 清理同一 runtime key，避免本地路由失败造成假 running。
- Note: 号池候选评分会读取最近 5 分钟的 `pool_upstream_request_attempts`，对最新仍处于 timeout/transport failure 的 `upstream_route_key + proxy_binding_key_snapshot` 组合增加短期排序惩罚；后续成功尝试会清除该短期惩罚。
- Note: `proxy capture follow-up` 的热路径门禁前移到 subscriber-aware 调度，避免无订阅者时每次代理收尾都触发重型 summary/quota 与 rollup 预算；对应回归测试与 `cargo check --tests` 已通过。
- Note: capture 入口的 request body 读取改为 replay snapshot 控制面；大 body 先落 file-backed replay snapshot，再进入现有完整 parse/rewrite/owner-binding 路径。超限错误只保留有界 partial body，避免 raw failure 证据回退且不重新制造整包内存副本。
- Note: file-backed capture snapshot 在进入现有 parse/rewrite 语义时只做一次 consume materialization；本轮没有把 capture pipeline 改成零拷贝 shared snapshot，因为这会牵涉 raw、failover、rewrite 与 terminal record 的共同数据模型。
- Note: file-backed pool routing 的 sticky key、prompt-cache key、model、encrypted、image 与 compaction 投影统一由一次 snapshot read / JSON parse 得出；解析后的完整 JSON 会立即释放，route/retry 分支只保留紧凑投影。sticky 投影字段的类型错误或重复字段继续降级为无 sticky 路由。慢日志带有 `file_read_count`、`json_parse_count`、`parse_outcome` 与 `analysis_elapsed_ms`，用于识别回归。
- Note: capture 请求统一等待完整 request 语义，并输出 `request_body_route_reason=complete_semantics_required`；同时保留 `body_size_bucket`、`request_body_snapshot_kind`、`downstream_first_byte_elapsed`、`raw_response_write_elapsed` 证据，供 101 判断剩余慢点。
- Note: pool failover replay snapshot 构造已收口到统一 helper：`Bytes` / `Vec<u8>` 小于等于 `POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES` 时保留 memory，大于阈值时写入 `cvm-pool-replay-*` 临时文件并返回 file snapshot；临时文件失败只 fail-soft 回退 memory 并输出 warning。
- Note: direct-image 路径使用独立首字节预算；超时后立即以 `504 upstream_handshake_timeout` 收口并释放 reservation，不进入 replay retry 或切号。
- Note: capture pool outbound 与 route-selection prebuffer fallback 不再直接为大 body 构造 `PoolReplayBodySnapshot::Memory(...)`；rewrite required 但 no-op 的分支保留原 file snapshot，真实 rewrite 后按同一阈值重新选择 memory/file。
- Note: `body_read_done/request_body_route_reason/request_body_snapshot_kind`、`downstream_first_byte_elapsed`、`raw_response_write_elapsed` 改为阈值化生产可见：大 body 或慢 body read、慢下游首字节、慢/大 raw response 在 `info` 输出，普通小请求继续保留 `debug`。
- Note: `/v1/responses` 流式解析器将完整、严格合法的 `response.completed` 视为协议成功终态；仍继续读取上游至 EOF 并收集 raw。终态后的上游读取异常、超时和已送达终态后的普通 body release 只写 payload 中性诊断，不覆盖成功、failure class、号池 attempt 或路由健康；终态前断连仍按 client abort 落盘。
- Note: 下游 body 的 watch 状态以独立的成功完成值保留已送达的协议终态，不能被随后 body EOF 覆盖；严格解析同时要求 SSE `event` 与 payload `type` 都是 `response.completed`。终态 chunk 被该 response body stream 成功取出即建立送达，普通 body release 不可反转这一状态。transport observer 继续记录实际 socket write error，但不会把共享 TCP 连接的写入归属到 HTTP/2 的特定 response；在成功终态之后观察到的错误只写 payload 中性诊断。终态前 body drop 仍按 `downstream_closed` 记录。

## 验证

- `cargo fmt --check`
- `cargo check --tests`
- `cargo test proxy_request_tracking_can_reach_100_in_flight_without_local_rejection -- --nocapture`
- `cargo test proxy_openai_v1_via_pool_reads_request_body_without_local_admission_gate -- --nocapture`
- `cargo test list_invocations_ -- --nocapture`
- `cargo test resolver_demotes_recent_timeout_for_same_upstream_route_and_proxy_binding -- --nocapture`
- `cargo test resolver_does_not_demote_successful_or_non_timeout_route_proxy_history -- --nocapture`
- `cargo test candidates_sort -- --nocapture`
- `scripts/shared-testbox-proxy-parallel-smoke`
- `cargo test stream_response_parser_recognizes_only_strict_successful_completion -- --nocapture`
- `cargo test pool_responses_ -- --nocapture`
- `cargo test skips_follow_up_without_subscribers -- --nocapture`
- `cargo test persist_and_broadcast_proxy_capture -- --nocapture`
- `cargo test acquire_proxy_request_concurrency_permit_tracks_multiple_in_flight_requests -- --nocapture`
- `cargo test acquire_proxy_request_concurrency_permit_tracks_100_in_flight_without_local_rejection -- --nocapture`
- `cargo test proxy_openai_v1_invalid_pool_key_bypasses_admission_backpressure -- --nocapture`
- `cargo test capture_snapshot_reader_ -- --nocapture`
- `cargo test pool_replay_snapshot_from_ -- --nocapture`
- `cargo test prepare_pool_request_body_for_account_ -- --nocapture`
- `cargo test capture_snapshot_reader_spills -- --nocapture`

## Migrated Task-Ticket Sections

## Task Orchestration

- wave: 1
  - main-agent => 从 `/v1/*` 入口移除 whole-proxy admission gate，把 permit 改成纯观测型 in-flight tracking，并清理 `PROXY_REQUEST_CONCURRENCY_*` 配置面 (skill: $fast-flow)
- wave: 2
  - main-agent => 保留并复验 request/response raw 异步旁路与 summary/quota debounce，确保移除 gate 后业务流优先级不回退 (skill: $fast-flow)
- wave: 3
  - main-agent => 补齐本地回归：100 并行不再本地 503、长流期间 in-flight tracking 到 body 结束、raw 饱和只丢 capture、不影响业务流 (skill: $fast-flow)
- wave: 4
  - main-agent => 新增 `scripts/shared-testbox-proxy-parallel-smoke`，在 `codex-testbox` 上用隔离 run dir、唯一 Compose project、LXC caps 兼容 override 跑 100 并行压测 (skill: $shared-testbox-runner)
- wave: 5
  - main-agent => 完成本地验证、共享测试机压测、fast-track review/PR/merge/release，并在 101 与浏览器侧复验不再出现本地 admission reject (skill: $fast-flow)

## Memory Attribution

- `MemoryDiagnosticsRuntime` 通过 raw writer semaphore occupancy 估算活跃压缩 writer 的 bounded ingress 保留量；不会读取 raw 文件目录，也不会在请求路径执行额外压缩或解压。
- allocator 诊断默认关闭。即使启用 `MEMORY_DIAGNOSTICS=allocator_once`，也仅在连续高未归因采样后生成一次受限文件，不包含 payload、调用 ID 或 SQL。
