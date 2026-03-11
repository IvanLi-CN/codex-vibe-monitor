# HTTP APIs

## `GET /api/pool/upstream-accounts`

返回账号列表：

```json
{
  "items": [
    {
      "id": 1,
      "kind": "oauth_codex",
      "provider": "codex",
      "displayName": "Work Pro",
      "status": "active",
      "enabled": true,
      "email": "user@example.com",
      "chatgptAccountId": "org_xxx",
      "planType": "pro",
      "lastSyncedAt": "2026-03-11T12:00:00Z",
      "lastSuccessfulSyncAt": "2026-03-11T12:00:00Z",
      "lastError": null,
      "primaryWindow": {
        "usedPercent": 42,
        "usedText": "42%",
        "limitText": "5h quota",
        "resetsAt": "2026-03-11T15:00:00Z",
        "windowDurationMins": 300
      },
      "secondaryWindow": {
        "usedPercent": 18,
        "usedText": "18%",
        "limitText": "7d quota",
        "resetsAt": "2026-03-17T00:00:00Z",
        "windowDurationMins": 10080
      },
      "credits": {
        "hasCredits": true,
        "unlimited": false,
        "balance": "9.99"
      }
    }
  ]
}
```

## `GET /api/pool/upstream-accounts/:id`

在 summary 基础上补充：

- `note`
- `chatgptUserId`
- `tokenExpiresAt`
- `lastRefreshedAt`
- `history`（最近 7 天样本）
- `localLimits`（API Key 账号）

## `POST /api/pool/upstream-accounts/oauth/login-sessions`

请求：

```json
{
  "displayName": "Work Pro",
  "note": "optional",
  "accountId": 1
}
```

- `accountId` 缺省时表示新建账号；存在时表示为现有账号重新登录。

响应：

```json
{
  "loginId": "c6e1f0f2d8f04fa9",
  "status": "pending",
  "authUrl": "https://auth.openai.com/oauth/authorize?...",
  "expiresAt": "2026-03-11T12:10:00Z",
  "accountId": null,
  "error": null
}
```

## `GET /api/pool/upstream-accounts/oauth/login-sessions/:loginId`

响应与上面一致；`status` 允许：`pending | completed | failed | expired`。

## `GET /api/pool/upstream-accounts/oauth/callback`

Query:

- `code`
- `state`
- `error`
- `error_description`

返回 HTML 页面，供弹窗显示/自动关闭；不返回 JSON。

## `POST /api/pool/upstream-accounts/api-keys`

请求：

```json
{
  "displayName": "Fallback Key",
  "note": "optional",
  "apiKey": "sk-...",
  "localPrimaryLimit": 200,
  "localSecondaryLimit": 2000,
  "localLimitUnit": "requests"
}
```

响应：返回新账号的 `UpstreamAccountDetail`。

## `PATCH /api/pool/upstream-accounts/:id`

支持更新：

- `displayName`
- `note`
- `enabled`
- `localPrimaryLimit`
- `localSecondaryLimit`
- `localLimitUnit`

## `DELETE /api/pool/upstream-accounts/:id`

- 成功返回 `204 No Content`。
- 同时删除关联的登录会话与历史样本。

## `POST /api/pool/upstream-accounts/:id/sync`

- OAuth：触发 refresh + usage sync。
- API Key：刷新本地状态时间戳。
- 成功返回同步后的 `UpstreamAccountDetail`。
