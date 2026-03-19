---
title: 配置参考
description: Codex Vibe Monitor 本地开发、文档站与部署相关的常用配置项。
---

# 配置参考

## 环境变量加载顺序

本地运行时建议把个人配置写入 `.env.local`。服务启动会以仓库根目录下的当前环境变量与 `.env.local` 为准。

## 核心运行时

- `HTTP_BIND`：后端监听地址，默认 `127.0.0.1:8080`
- `DATABASE_PATH`：SQLite 主库路径，默认 `codex_vibe_monitor.db`
- `STATIC_DIR`：若存在则用于托管前端静态产物，默认 `web/dist`
- `VITE_BACKEND_PROXY`：前端开发服务器代理目标，默认 `http://localhost:8080`

## 代理捕获与上游请求

- `OPENAI_UPSTREAM_BASE_URL`：OpenAI 兼容上游基址
- `REQUEST_TIMEOUT_SECS`：通用请求超时
- `OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS`：非 compact 路径的上游握手超时
- `OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS`：`/v1/responses/compact` 上游握手超时
- `OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS`：请求体读取总超时
- `OPENAI_PROXY_MAX_REQUEST_BODY_BYTES`：请求体最大尺寸限制
- `PROXY_RAW_DIR` / `PROXY_RAW_MAX_BYTES` / `PROXY_RAW_COMPRESSION` / `PROXY_RAW_HOT_SECS`：原始 payload 落盘、热保留与冷压缩策略
- `PROXY_ENFORCE_STREAM_INCLUDE_USAGE`：是否在流式请求中强制补 `include_usage`
- `PROXY_USAGE_BACKFILL_ON_STARTUP`：历史补数兼容开关
- `FORWARD_PROXY_ALGO`：forward proxy 权重算法版本

## 上游账号与号池

- `UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET`：启用号池写入与加密落库的必填密钥
- `UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID`
- `UPSTREAM_ACCOUNTS_OAUTH_ISSUER`
- `UPSTREAM_ACCOUNTS_USAGE_BASE_URL`
- `UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS`
- `UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS`
- `UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS`
- `UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS`

## 归档与保留

- `RETENTION_ENABLED`
- `RETENTION_DRY_RUN`
- `RETENTION_INTERVAL_SECS`
- `RETENTION_BATCH_ROWS`
- `ARCHIVE_DIR`
- `INVOCATION_SUCCESS_FULL_DAYS`
- `INVOCATION_MAX_DAYS`
- `FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS`
- `STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS`
- `QUOTA_SNAPSHOT_FULL_DAYS`

这些参数控制在线明细、离线 archive 与定时 maintenance 行为。更深的部署与安全边界说明请回到仓库中的 `docs/deployment.md`。

## 文档站与 Storybook

- `DOCS_PORT`：docs-site 本地 dev/preview 端口，默认 `60081`
- `DOCS_BASE`：静态站部署基路径；GitHub Pages 项目页通常使用 `/<repo>/`
- `VITE_STORYBOOK_DEV_ORIGIN`：docs-site 本地 `storybook.html` 跳转到 Storybook dev server 时使用的完整 origin；默认 `http://127.0.0.1:60082`
- `STORYBOOK_PORT`：Storybook 本地开发端口，默认 `60082`

## 端口合同

- App dev：`60080`
- docs-site dev / preview：`60081`
- Storybook dev：`60082`
- Backend：`8080`

这些端口都允许通过 env 或命令行覆盖，但文档、脚本与 CI 默认按以上合同组织。
