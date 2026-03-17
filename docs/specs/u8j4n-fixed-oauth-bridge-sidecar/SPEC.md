# 固定 OAuth Bridge Sidecar 方案（#u8j4n）

## 状态

- Status: 重新设计（#pd77h）
- Created: 2026-03-16
- Last: 2026-03-16

## 变更说明

- 当前方案已被 `#pd77h` 的单进程 OAuth adapter 方案取代，不再作为实现依据。

## 背景 / 问题陈述

- 官方 `auth.openai.com` OAuth client 不允许申请 `api.model.read` / `api.responses.write`，因此“补 API scopes 后让 OAuth token 直连 `api.openai.com`”这条路线不可行。
- 现有 OAuth 账号可以完成登录与 usage 同步，但要承载 `/v1/*` 数据面，必须经过一层固定桥接，把本地持有的官方 access token 变成可供池路由使用的短期 bridge token。
- 主服务此前把 OAuth 账号与 API key 账号混用同一上游配置，也会把部分 bridge/上游失败误判成 `needs_reauth`，导致 UI 和运维动作都被误导。

## 目标 / 非目标

### Goals

- OAuth 登录回退到官方允许的基础 scopes：`openid profile email offline_access`，保留现有 audience / prompt / PKCE / refresh 流程。
- 项目内提供固定搭配 sidecar `ai-openai-oauth-bridge`，主服务对 OAuth 账号固定走 `http://ai-openai-oauth-bridge:3000/internal/token/register` 与 `http://ai-openai-oauth-bridge:3000/openai`。
- API key 账号继续沿用现有 `OPENAI_UPSTREAM_BASE_URL` 与账号级 `upstreamBaseUrl` 覆盖逻辑，不受这次改动影响。
- bridge 不可达、token register 失败、bridge 数据面 401/403/5xx 统一落 `error`；只有 refresh/token 端明确失效才落 `needs_reauth`。
- UI / README / deployment 文档统一改口径，删除“补 API scopes / 重新授权拿新 scopes”的说法，改成 fixed bridge sidecar 语义。

### Non-goals

- 不新增任何 bridge 用户配置项、账号级 bridge 覆盖项或新的 OAuth provider。
- 不把 bridge token 持久化到数据库。
- 不改 API key 路由、forward proxy 或其它非 OAuth 账号逻辑。

## 需求（Requirements）

### MUST

- OAuth authorize URL 必须只包含 `openid profile email offline_access`，并保留 `audience=https://api.openai.com/v1`、`prompt=login`、PKCE 参数。
- 主服务在 OAuth 账号参与池路由前，必须先用本地 refresh/access token 完成固定 `POST /internal/token/register`，再用返回的 `token_key` 调用固定 `/openai/v1/*`。
- 固定 bridge sidecar 必须随项目交付，提供 `GET /health`、`POST /internal/token/register`、`GET /openai/v1/models`、`POST /openai/v1/responses`、`POST /openai/v1/responses/compact`、`POST /openai/v1/chat/completions`。
- bridge token 仅允许进程内缓存，并按 `account_id + access_token 指纹 + expire_at` 失效重换。
- OAuth 路由阶段只有 refresh/token 端明确 `invalid_grant`、refresh token revoked/expired、token invalidated 等场景才允许把账号标成 `needs_reauth`。
- Docker 镜像必须同时包含 `codex-vibe-monitor` 与 `openai-oauth-bridge` 两个可执行文件，方便同镜像双服务部署。

## 验收标准（Acceptance Criteria）

- OAuth 登录回调不再出现 `invalid_scope`。
- OAuth 账号的池路由不再复用 `OPENAI_UPSTREAM_BASE_URL`，而是固定走 `ai-openai-oauth-bridge:3000/openai`。
- bridge token register 失败或 bridge 数据面拒绝请求时，账号会进入 `error` 并保留 bridge 相关错误摘要，而不是误标成 `needs_reauth`。
- refresh/token 明确失效时，账号仍会进入 `needs_reauth`。
- Rust / Web 测试覆盖 authorize URL 回退、固定 bridge 路由、bridge 故障归类、UI bridge 恢复提示，以及 bridge sidecar 丢注册后的自动重注册恢复。

## 实现备注

- 参考 `claude-relay-service` 的固定 sidecar / 同网络服务名 / `/openai` 数据面部署模式，但不暴露任何用户配置项。
- bridge 当前使用 `chatgpt.com/backend-api/codex` 作为内部数据面来源，并在 sidecar 内做最小 OpenAI-compatible 适配。
