# 事件（Events）

## summary（SSE）

- Producer: backend
- Consumers: web
- Delivery: SSE（可能重复发送；消费端需幂等处理）
- Change: Modify（口径改为多来源合并）

### Payload schema

```json
{
  "type": "summary",
  "window": "all|30m|1h|1d|1mo|...",
  "summary": {
    "totalCount": 0,
    "successCount": 0,
    "failureCount": 0,
    "totalCost": 0,
    "totalTokens": 0
  }
}
```

### Notes

- 结构不变，数值为合并口径。
- 外部源不提供明细，因此 `records` 事件仍仅覆盖已有来源。
