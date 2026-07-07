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
