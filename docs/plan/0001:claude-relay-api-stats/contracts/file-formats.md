# 文件格式（File formats）

## `.env.local`（新增/更新配置项）

- 变更（Change）: Modify
- 范围（Scope）: internal

### Proposed keys（待确认）

```env
# CRS 日统计源
CRS_STATS_BASE_URL=https://claude-relay-service.nsngc.org
CRS_STATS_API_ID=<apiId>
CRS_STATS_PERIOD=daily
CRS_STATS_POLL_INTERVAL_SECS=10
```

### Notes

- 目前抓包显示无需鉴权；如后续需要，可新增可选鉴权配置项。
- `CRS_STATS_PERIOD` 暂仅支持 `daily`（若启用 monthly 需明确产品口径）。
