# 代理热路径并发稳定性与传输背压收口 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/q8h3n-proxy-hot-path-streaming-stability/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.
- 号池路由使用组合级短期降权，以 `pool_upstream_request_attempts.upstream_route_key + proxy_binding_key_snapshot` 作为传输组合键；仅 timeout/transport/stream failure 触发，后续成功清除惩罚，避免把账号硬失败误当作代理组合问题。
- 2026-07-05: `/v1/*` 本地并发语义收口为纯观测：`PROXY_REQUEST_CONCURRENCY_*` 不参与 admission、raw writer sizing 或新失败分类。tracked 请求在 route 前创建内存 running shell，并用 terminal overlay 收敛失败路径。
- 2026-07-07: `PROXY_REQUEST_CONCURRENCY_*` 配置面从 active code 清理，不再读取、告警或暴露为 `AppConfig` 字段；历史 `proxy_concurrency_limit` failure kind 继续保留用于旧记录统计兼容。
- 2026-07-05: capture 转发提速先按“不劣化功能”收口：大 body 读取切到 replay snapshot/file-backed 控制面，并补齐 live-first fallback 与响应首字节/raw writer 耗时证据；未在本轮强开可能破坏 encrypted owner、prompt-cache binding、rewrite 或 failover replay 的 capture live-first。
- 2026-07-05: 101 线上证据显示 11MB/21MB/62MB 请求在 timeout 日志中仍有 `snapshot_kind="memory"`，说明直接从完整 body 构造 memory replay 的残留路径未收口。本轮把 `Bytes` / `Vec<u8>` 到 replay snapshot 的转换统一到阈值 helper，capture outbound、route-selection prebuffer fallback、rewrite changed 都复用该 helper；rewrite no-op 保留原 snapshot，避免 file-backed snapshot 被无意义重新物化为 memory。
- 2026-07-05: 生产排障证据从 debug-only 调整为阈值化 info：普通小请求不刷屏，但大 body、慢 body read、慢 downstream first byte、慢/大 raw response write 在默认 info 日志下可见，避免把“没有 debug 日志”误判成没有埋点。
- 2026-07-14: Direct-image 首字节超时改为单次、不可重试终态，返回 `504 upstream_handshake_timeout`；这避免重复图片任务与计费，也不再把真实 timeout 掩盖成无可用账号。
- SSE 的协议成功终态优先于传输 EOF：严格合法的 `response.completed` 一旦实际送达下游，后续上游读取异常和普通 body release 只能保留诊断，不得倒灌为服务失败或下游失败。
- 2026-07-26: 下游 body EOF 不能覆盖已送达的成功终态；以单调的成功完成状态传递给对应 response body 的所有 watch 接收方。解析器同时要求 `event` 与 payload `type` 完整匹配。终态 chunk 被下游 body stream 成功取出即建立协议送达，不以共享 TCP 连接的写入结果反推 HTTP/2 中某个 response 的状态；之后观察到的 socket error 只保留在 payload 诊断字段。
- file-backed pool replay 的路由投影收口为单次读取与单次 JSON parse；解析结果限定为路由所需的紧凑字段，保留既有 sticky key 类型错误和重复字段降级语义，避免大请求在同一准备阶段重复打开临时文件。
- Response raw storage distinguishes wire encoding from storage encoding: identity content is stored with Zstd, while pre-compressed wire bytes remain untouched. A saturated writer uses a CRC-framed local spool so enabled response capture is not silently discarded.
- Paired pool response capture writes one finalized payload and stores independent invocation/attempt links. Durable spool capacity failure is explicit `capture_unavailable`, avoiding a second in-memory queue and duplicate response compression.
- Raw codec inference records its own completion marker atomically with legacy backfill. The pre-existing raw-blob link seed marker is accepted only as compatibility proof because schema startup has already completed codec inference before that seed is recorded.

- response raw 的内存归因与物理文件指标分开：writer occupancy 通过现有 semaphore 和有界队列估算，spool 只作为持久化/磁盘指标；不会因为 raw 文件总量或 `liveInvocationsCount` 直接判定 RSS 根因。
