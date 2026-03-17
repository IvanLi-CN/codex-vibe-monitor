# OAuth 池账号 API Scope 与重授权误判修复（#r7o2q）

## 状态

- Status: 已废弃（被固定 OAuth bridge sidecar 方案取代）
- Created: 2026-03-16
- Last: 2026-03-16

## 背景 / 问题陈述

- 这份规格最初假设“给官方 OAuth 登录补 API scopes 后即可直连 `api.openai.com`”，但现网回调已经确认官方 `auth.openai.com` client 不允许请求 `api.model.read` / `api.responses.write`。
- 因此这条路线已经失效，不能再作为实现依据；后续实现统一转到固定 OAuth bridge sidecar 方案。

## 目标 / 非目标

### Goals

- 仅保留可继续复用的误判收紧思路：OAuth 路由阶段只有在明确凭据失效时才转 `needs_reauth`。
- 其余关于 API scopes 直连 `api.openai.com` 的目标全部作废。

### Non-goals

- 不做数据库批量回填，也不尝试自动让旧 token 获得新增 scopes。
- 不改变 API Key 账号的路由与状态机语义。
- 不修改 101 的运行配置或部署拓扑。

## 需求（Requirements）

### MUST

- 本节原有 requirements 已作废，不再作为实现约束。
- 唯一保留的有效方向是：OAuth 路由阶段的 `401/403` 只有在错误文本明确指向 `invalid_grant`、token invalidated、必须重新登录等场景时，才可落 `needs_reauth`。

## 验收标准（Acceptance Criteria）

- 本节原有 acceptance 已作废，替代验收标准以固定 OAuth bridge sidecar 方案为准。

## 实现备注

- 新实现请改看替代规格：固定 OAuth bridge sidecar 方案。
