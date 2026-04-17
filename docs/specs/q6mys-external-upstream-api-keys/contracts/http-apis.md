# HTTP API

## External API keys management（GET `/api/settings/external-api-keys`）

- 范围（Scope）: internal
- 变更（Change）: New
- 鉴权（Auth）: none（沿用当前设置页读取语义）

### 请求（Request）

- Headers: none
- Body: none

### 响应（Response）

- Success:
  ```json
  {
    "items": [
      {
        "id": 12,
        "name": "Provider A",
        "status": "active",
        "prefix": "cvm_ext_ab12",
        "lastUsedAt": "2026-04-17T10:20:30Z",
        "createdAt": "2026-04-17T09:00:00Z",
        "updatedAt": "2026-04-17T09:00:00Z"
      }
    ]
  }
  ```
- Error: plain-text `500`

## Create external API key（POST `/api/settings/external-api-keys`）

- 范围（Scope）: internal
- 变更（Change）: New
- 鉴权（Auth）: browser same-origin settings write（要求有效 `Origin`）

### 请求（Request）

- Headers: same-origin settings write headers
- Body:
  ```json
  {
    "name": "Provider A"
  }
  ```

### 响应（Response）

- Success:
  ```json
  {
    "key": {
      "id": 12,
      "name": "Provider A",
      "status": "active",
      "prefix": "cvm_ext_ab12",
      "lastUsedAt": null,
      "createdAt": "2026-04-17T09:00:00Z",
      "updatedAt": "2026-04-17T09:00:00Z"
    },
    "secret": "cvm_ext_ab12_..."
  }
  ```
- Error: plain-text `400|403|409|500`

## Rotate external API key（POST `/api/settings/external-api-keys/:id/rotate`）

- 范围（Scope）: internal
- 变更（Change）: New
- 鉴权（Auth）: browser same-origin settings write（要求有效 `Origin`）

### 响应（Response）

- Success: same shape as create；返回新 key 元数据与一次性 `secret`
- Error: plain-text `403|404|409|500`

### 兼容性与迁移（Compatibility / migration）

- 旧 secret 在 rotate 成功后立即失效，旧 row 标记为 `rotated` 并从默认列表中隐藏。
- disabled key 也允许 rotate；rotate 后会发放新的 active replacement key，并保留原有 `client_id` 绑定。
- 新 row 继承稳定 `client_id`，因此外部账号归属不会改变。

## Disable external API key（POST `/api/settings/external-api-keys/:id/disable`）

- 范围（Scope）: internal
- 变更（Change）: New
- 鉴权（Auth）: browser same-origin settings write（要求有效 `Origin`）

### 响应（Response）

- Success:
  ```json
  {
    "key": {
      "id": 12,
      "name": "Provider A",
      "status": "disabled",
      "prefix": "cvm_ext_ab12",
      "lastUsedAt": "2026-04-17T10:20:30Z",
      "createdAt": "2026-04-17T09:00:00Z",
      "updatedAt": "2026-04-17T11:00:00Z"
    }
  }
  ```
- Error: plain-text `403|404|500`

## External OAuth upstream upsert（PUT `/api/external/v1/upstream-accounts/oauth/{sourceAccountId}`）

- 范围（Scope）: external
- 变更（Change）: New
- 鉴权（Auth）: `Authorization: Bearer <external_api_key_secret>`

### 请求（Request）

- Path: `sourceAccountId`（第三方来源稳定主键）
- Body:
  ```json
  {
    "displayName": "Provider A / alpha@example.com",
    "groupName": "alpha",
    "groupBoundProxyKeys": ["DIRECT"],
    "groupNodeShuntEnabled": false,
    "note": "managed by provider",
    "groupNote": "provider managed",
    "concurrencyLimit": 2,
    "enabled": true,
    "isMother": false,
    "tagIds": [1, 2],
    "oauth": {
      "email": "alpha@example.com",
      "accessToken": "at_...",
      "refreshToken": "rt_...",
      "idToken": "eyJ...",
      "tokenType": "Bearer",
      "expired": "2026-04-18T00:00:00Z"
    }
  }
  ```

### 响应（Response）

- Success: `UpstreamAccountDetail`（仅返回当前 client 绑定的目标账号）
- Error: plain-text `400|401|403|404|409|422|500`

### 兼容性与迁移（Compatibility / migration）

- 幂等键固定为 `external_client_id + sourceAccountId`。
- 若未显式提交 metadata，则更新凭据时保留现有 metadata。

## External OAuth upstream metadata patch（PATCH `/api/external/v1/upstream-accounts/oauth/{sourceAccountId}`）

- 范围（Scope）: external
- 变更（Change）: New
- 鉴权（Auth）: Bearer external API key

### 请求（Request）

- Body: 与 upsert 相同的 metadata 字段子集，但不包含 `oauth`

### 响应（Response）

- Success: `UpstreamAccountDetail`
- Error: plain-text `400|401|403|404|409|500`

### 兼容性与迁移（Compatibility / migration）

- PATCH 只覆盖请求体中出现的 metadata 字段；OAuth 凭据保持不变。

## External OAuth upstream relogin repair（POST `/api/external/v1/upstream-accounts/oauth/{sourceAccountId}/relogin`）

- 范围（Scope）: external
- 变更（Change）: New
- 鉴权（Auth）: Bearer external API key

### 请求（Request）

- Body:
  ```json
  {
    "oauth": {
      "email": "alpha@example.com",
      "accessToken": "at_...",
      "refreshToken": "rt_...",
      "idToken": "eyJ...",
      "tokenType": "Bearer",
      "expired": "2026-04-18T00:00:00Z"
    }
  }
  ```

### 响应（Response）

- Success: `UpstreamAccountDetail`
- Error: plain-text `400|401|403|404|422|500`

### 兼容性与迁移（Compatibility / migration）

- relogin 不返回 authUrl；它直接把新 OAuth 凭据写回并触发同步修复。
