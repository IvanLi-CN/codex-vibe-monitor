# 上游账号模型映射 HTTP 契约

## 保存映射

`PUT /api/pool/upstream-accounts/:id/model-mappings`

请求体：

```json
{
  "modelMappings": [
    {
      "sourceModel": "gpt-5-*",
      "targetModel": "gpt-5.4-mini",
      "enabled": true
    }
  ]
}
```

- 使用现有账号写入鉴权和 same-origin guard。
- 列表整体替换，保留数组顺序。
- 成功返回完整更新后的 `UpstreamAccountDetail`。
- 验证失败返回现有 4xx 错误格式，且不部分保存任何条目。

## 详情与尝试字段

`UpstreamAccountDetail` 增加：

```json
{
  "modelMappings": [
    { "sourceModel": "gpt-5-*", "targetModel": "gpt-5.4-mini", "enabled": true }
  ]
}
```

账户尝试对象增加：

```json
{
  "requestModel": "gpt-5-preview",
  "upstreamRequestModel": "gpt-5.4-mini",
  "modelMappingPattern": "gpt-5-*"
}
```

`upstreamRequestModel` 为空表示上游请求尚未发送。历史记录没有该列时，API 以 `requestModel` 作为显示回退。
