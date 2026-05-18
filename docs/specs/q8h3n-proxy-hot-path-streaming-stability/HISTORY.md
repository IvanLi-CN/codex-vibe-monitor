# 代理热路径并发稳定性与传输背压收口 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/q8h3n-proxy-hot-path-streaming-stability/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.
- 号池路由使用组合级短期降权，以 `pool_upstream_request_attempts.upstream_route_key + proxy_binding_key_snapshot` 作为传输组合键；仅 timeout/transport/stream failure 触发，后续成功清除惩罚，避免把账号硬失败误当作代理组合问题。
