# HTTP APIs

## `GET /api/pool/upstream-accounts`

支持列表筛选 query 参数：

- `groupSearch`：按分组名模糊匹配；空值表示不过滤。
- `groupUngrouped=true`：只返回未分组账号。
- `workStatus=working|idle|rate_limited`：按系统工作状态筛选；支持重复参数，同一维度内按 OR 匹配，例如 `workStatus=working&workStatus=rate_limited`。只有 `enableStatus=enabled`、`healthStatus=normal` 且 `syncState=idle` 的账号才可能返回 `working` 或 `rate_limited`，其它账号统一返回 `idle`。
- `enableStatus=enabled|disabled`：按启用状态筛选；支持重复参数，同一维度内按 OR 匹配。
- `healthStatus=normal|needs_reauth|upstream_unavailable|upstream_rejected|error_other`：按账号固有健康状态筛选；支持重复参数，同一维度内按 OR 匹配。
- `tagIds=1&tagIds=2...`：标签多选全匹配；只有同时包含全部已选 tag 的账号才返回。
- `status=...`：旧状态参数兼容一轮；`active -> healthStatus=normal`、`disabled -> enableStatus=disabled`、`syncing -> syncState=syncing`，其它旧异常值映射到对应 `healthStatus`。单值 `workStatus|enableStatus|healthStatus` 仍兼容，因为服务端会把它们视为长度为 `1` 的重复参数集合。

上述筛选均由后端执行；前端只负责透传当前筛选状态。`syncState` 不提供主筛选参数，只作为响应里的次级过程状态返回。

列表与详情共享 `activeConversationCount` 字段：表示最近 `30` 分钟内仍活跃的 sticky route 数量；缺省返回 `0`。

返回账号列表：

```json
{
  "items": [
    {
      "id": 1,
      "kind": "oauth_codex",
      "provider": "codex",
      "displayName": "Work Pro",
      "groupName": "production",
      "isMother": true,
      "status": "active",
      "displayStatus": "active",
      "workStatus": "working",
      "enableStatus": "enabled",
      "healthStatus": "normal",
      "syncState": "idle",
      "enabled": true,
      "email": "user@example.com",
      "chatgptAccountId": "org_xxx",
      "duplicateInfo": {
        "peerAccountIds": [2],
        "reasons": ["sharedChatgptAccountId"]
      },
      "planType": "pro",
      "lastSyncedAt": "2026-03-11T12:00:00Z",
      "lastSuccessfulSyncAt": "2026-03-11T12:00:00Z",
      "activeConversationCount": 3,
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
  ],
  "hasUngroupedAccounts": true
}
```

## `GET /api/pool/upstream-accounts/:id`

在 summary 基础上补充：

- 保留 `workStatus`、`enableStatus`、`healthStatus`、`syncState` 四个读模型状态字段
- `activeConversationCount`（最近 `30` 分钟活跃 sticky route 数量，缺省为 `0`）
- `note`
- `chatgptUserId`
- `tokenExpiresAt`
- `lastRefreshedAt`
- `history`（最近 7 天样本）
- `localLimits`（API Key 账号）
- `duplicateInfo`（仅 OAuth；共享 `chatgptAccountId` / `chatgptUserId` 时返回 warning 信息）

`isMother` 表示该账号是否为所在分组的母号；同一分组最多只能有一个母号，未分组账号视为同一个分组。

## `POST /api/pool/upstream-accounts/oauth/login-sessions`

请求：

```json
{
  "displayName": "Work Pro",
  "note": "optional",
  "accountId": 1,
  "groupName": "production",
  "isMother": true
}
```

- `accountId` 缺省时表示新建账号；存在时表示为现有账号重新登录。
- `isMother=true` 时，callback 落库会自动把同组旧母号降级为非母号。

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
  "groupName": "production",
  "note": "optional",
  "isMother": true,
  "apiKey": "sk-...",
  "localPrimaryLimit": 200,
  "localSecondaryLimit": 2000,
  "localLimitUnit": "requests"
}
```

响应：返回新账号的 `UpstreamAccountDetail`。

- 若 `displayName` 与现有账号重复（忽略大小写 + 去首尾空格），返回 `409 Conflict`，消息为 `displayName must be unique`。

## `PATCH /api/pool/upstream-accounts/:id`

支持更新：

- `displayName`
- `groupName`
- `note`
- `enabled`
- `isMother`
- `localPrimaryLimit`
- `localSecondaryLimit`
- `localLimitUnit`

- 若更新后的 `displayName` 与其他账号重复（忽略大小写 + 去首尾空格），返回 `409 Conflict`，消息为 `displayName must be unique`。
- 服务端必须与同账号的后台维护、手动同步、删除、re-login callback 落库和导入覆盖串行执行；无关账号之间不得互相阻塞。

## OAuth 完成的重复 warning

- `POST /api/pool/upstream-accounts/oauth/login-sessions/:loginId/complete` 成功时仍返回 `UpstreamAccountDetail`。
- 若该 OAuth 账号与其他 OAuth 账号共享 `chatgptAccountId` 或 `chatgptUserId`，响应中的 `duplicateInfo` 会带出 warning；这不会阻止保存。
- 只有显式 `accountId` 的 re-login 会更新既有账号；新建 OAuth 不再按 `chatgptAccountId` 合并旧记录。
规则：

- `isMother=true` 会自动撤销同组其他账号的母号标记。
- `isMother=false` 只清除当前账号的母号标记，不会自动提升其他账号。

## `DELETE /api/pool/upstream-accounts/:id`

- 成功返回 `204 No Content`。
- 同时删除关联的登录会话与历史样本。
- 服务端必须与同账号的后台维护、启停、保存、手动同步和导入覆盖串行执行。

## `POST /api/pool/upstream-accounts/:id/sync`

- OAuth：触发 refresh + usage sync。
- API Key：刷新本地状态时间戳。
- 成功返回同步后的 `UpstreamAccountDetail`。
- 同账号存在运行中或排队中的后台维护时，该同步请求必须排队执行；后台维护不得阻塞无关账号的 `PATCH enabled` / `PATCH disabled`。

## 后台维护并发保证

- 后台维护只允许按账号去重；同一账号存在运行中或排队中的维护任务时，新的维护请求必须被合并。
- 在后台维护竞争下，无关账号的人工启用/禁用请求必须以 `1 秒内完成服务端提交` 为目标，不允许因为整池维护扫描而长时间 pending。
