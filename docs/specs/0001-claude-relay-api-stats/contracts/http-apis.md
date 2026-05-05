# HTTP API

## 外部日统计源（POST /apiStats/api/user-model-stats）

- 范围（Scope）: external
- 变更（Change）: New
- 鉴权（Auth）: none（2026-01-16 抓包观察）
- Base URL: `https://claude-relay-service.nsngc.org`

### 请求（Request）

- Headers:
  - `Content-Type: application/json`
- Body:
  - `apiId` (string, required): API Key 标识
  - `period` (string, required): `daily`（当前仅使用该值；`monthly` 观察到存在但不在本计划范围）

### 响应（Response）

- Success:
  - `success` (boolean)
  - `data` (array of ModelStats)
  - `period` (string)
- ModelStats:
  - `model` (string)
  - `requests` (number)
  - `inputTokens` (number)
  - `outputTokens` (number)
  - `cacheCreateTokens` (number)
  - `cacheReadTokens` (number)
  - `allTokens` (number)
  - `costs` (object): `input`, `output`, `cacheWrite`, `cacheRead`, `total`
  - `formatted` (object): 同上但为字符串
  - `pricing` (object): `input`, `output`, `cacheWrite`, `cacheRead`
- Error: `{ success: false, message?: string }` 或 HTTP 4xx/5xx（以实际响应为准）

### 错误（Errors）

- 429: 触发限流（retryable: yes）
- 5xx: 上游错误（retryable: yes）

### 示例（Examples）

- Request（请求）:

```json
{ "apiId": "<redacted>", "period": "daily" }
```

- Response（响应）:

```json
{
  "success": true,
  "data": [
    {
      "model": "<model>",
      "requests": 0,
      "inputTokens": 0,
      "outputTokens": 0,
      "cacheCreateTokens": 0,
      "cacheReadTokens": 0,
      "allTokens": 0,
      "costs": { "input": 0, "output": 0, "cacheWrite": 0, "cacheRead": 0, "total": 0 },
      "formatted": { "input": "$0.00", "output": "$0.00", "cacheWrite": "$0.00", "cacheRead": "$0.00", "total": "$0.00" },
      "pricing": { "input": 0, "output": 0, "cacheWrite": 0, "cacheRead": 0 }
    }
  ],
  "period": "daily"
}
```

### 兼容性与迁移（Compatibility / migration）

- 若字段缺失或命名不同，按抓包结果更新映射与契约文件。

---

## 统计接口（GET /api/stats）

- 范围（Scope）: internal
- 变更（Change）: Modify（口径改为多来源合并）
- 鉴权（Auth）: none

### 请求（Request）

- Headers: none
- Query: none
- Body: none

### 响应（Response）

- Success:
  - `totalCount` (i64)
  - `successCount` (i64)
  - `failureCount` (i64)
  - `totalCost` (f64)
  - `totalTokens` (i64)
- Error: `{ error: string }`（与现有错误返回保持一致）

### 错误（Errors）

- 500: 统计查询失败（retryable: yes）

### 示例（Examples）

- Response（响应）:
  - `{ "totalCount": 0, "successCount": 0, "failureCount": 0, "totalCost": 0, "totalTokens": 0 }`

### 兼容性与迁移（Compatibility / migration）

- 返回结构不变，仅统计口径变为“合并来源”。

---

## 统计接口（GET /api/stats/summary）

- 范围（Scope）: internal
- 变更（Change）: Modify（口径改为多来源合并）
- 鉴权（Auth）: none

### 请求（Request）

- Query:
  - `window` (string, optional): `all|current|30m|1h|1d|1mo|today|thisWeek|thisMonth|...`
  - `limit` (number, optional): `window=current` 时生效

### 响应（Response）

- Success: 同 `/api/stats`
- Error: `{ error: string }`

### 兼容性与迁移（Compatibility / migration）

- 返回结构不变，窗口内统计合并来源；外部源仅影响其可用时段。

---

## 统计接口（GET /api/stats/timeseries）

- 范围（Scope）: internal
- 变更（Change）: Modify（口径改为多来源合并）
- 鉴权（Auth）: none

### 请求（Request）

- Query:
  - `range` (string, required): `1h|6h|1d|7d|1mo|today|thisWeek|thisMonth|...`
  - `bucket` (string, optional): `1m|5m|15m|1h|6h|1d|...`
  - `settlement_hour` (number, optional)

### 响应（Response）

- Success:
  - `rangeStart` (ISO timestamp)
  - `rangeEnd` (ISO timestamp)
  - `bucketSeconds` (number)
  - `points[]` with:
    - `bucketStart` / `bucketEnd`
    - `totalCount` / `successCount` / `failureCount`
    - `totalTokens` / `totalCost`
- Error: `{ error: string }`

### 兼容性与迁移（Compatibility / migration）

- 返回结构不变，内部实现将外部源的增量统计合并到桶中。
