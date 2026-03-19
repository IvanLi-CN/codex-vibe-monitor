---
title: 配置参考
description: 先覆盖自部署常用运行参数，再补充开发和文档站配置合同。
---

# 配置参考

## 如果你是第一次部署，先决定这几件事

| 关注点 | 关键变量 | 默认值 | 什么时候该改 |
| --- | --- | --- | --- |
| 应用监听地址 | `HTTP_BIND` | 本地默认 `127.0.0.1:8080`；运行镜像默认 `0.0.0.0:8080` | 本地开发、容器部署或反向代理拓扑不同时 |
| 数据库存放位置 | `DATABASE_PATH` | `codex_vibe_monitor.db` | 想把 SQLite 和归档放到持久化卷时 |
| 上游代理目标 | `OPENAI_UPSTREAM_BASE_URL` | OpenAI 官方默认地址 | 你接的是自建兼容上游或其他转发层时 |
| 账号池写能力 | `UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET` | 无 | 只要你要新增账号、改账号或用 OAuth 登录，就必须配置 |
| 数据保留与归档 | `RETENTION_ENABLED`、`ARCHIVE_DIR`、各类 retention 天数 | 默认偏保守、默认不开启后台维护 | 想长期运行并控制主库体积时 |

如果你只是先把服务跑起来，通常先确认上表这 4 到 5 项就够了。

## 自部署最常见的运行时变量

### 基础运行

- `HTTP_BIND`：后端监听地址
- `DATABASE_PATH`：SQLite 主库路径
- `STATIC_DIR`：若存在则用于托管前端静态产物，默认 `web/dist`
- `USER_AGENT`：出站请求的默认 UA
- `CORS_ALLOWED_ORIGINS`：只有需要显式跨域时才配置

### 代理捕获与上游请求

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

### 上游账号与号池

- `UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET`：启用号池写入与加密落库的必填密钥
- `UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID`
- `UPSTREAM_ACCOUNTS_OAUTH_ISSUER`
- `UPSTREAM_ACCOUNTS_USAGE_BASE_URL`
- `UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS`
- `UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS`
- `UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS`
- `UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS`

如果你不打算启用 Account Pool 的写入能力，这一组变量可以后置；如果要走 OAuth 登录或账号写入，这一组不要拖到最后才补。

### 归档与保留

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

这些参数控制在线明细、离线 archive 与后台 maintenance 行为。更细的部署与安全边界请回到仓库里的 [Deployment Guide](https://github.com/IvanLi-CN/codex-vibe-monitor/blob/main/docs/deployment.md)。

## 开发与文档站相关变量

- `VITE_BACKEND_PROXY`：前端开发服务器代理目标，默认 `http://localhost:8080`
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

## 本地环境加载

本地运行时建议把个人配置写入 `.env.local`。服务启动会以仓库根目录下的当前环境变量与 `.env.local` 为准。
