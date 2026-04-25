# History

- 2026-04-24：创建 `#ngwdu`，冻结“owner-facing 节点健康统一为真实节点尝试口径”的规范边界。
- 2026-04-24：确认本 spec supersedes `#3np57` 的 owner-facing group-scoped binding-node 统计语义，但保留其快照字段作为真实节点尝试历史真相源。
- 2026-04-24：确认 `#t7m4h` 的权重趋势 fallback 继续有效，本轮只修请求成功/失败口径，不改权重语义。
- 2026-04-24：补上 `pool_upstream_node_health_hourly_archive` 长存小时桶，确保 raw archive TTL 清理后 `forward-proxy/timeseries` 的 `90d` 历史仍保持真实节点尝试口径，不回退到 `forward_proxy_attempt_hourly`。
- 2026-04-24：修正 retention append 既有月归档时的 replay marker 语义，避免把 rollout 前旧月文件在只追加新 live rows 后误标为 fully replayed，从而漏掉完整 backfill。
- 2026-04-24：修正 forward-proxy timeseries 对 retired Direct 的保留逻辑；即便关闭 `insert_direct`，历史 `__direct__` 节点尝试仍保留在 timeseries 中。
