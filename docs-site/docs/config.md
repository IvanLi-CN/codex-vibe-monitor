---
title: 配置与运行
description: 先覆盖首次部署真正要决定的参数，再说明账号池、归档和开发口径。
---

# 配置与运行

这页不只是环境变量清单，而是帮你先决定三件事：

1. 第一次部署最少要配哪些参数
2. 哪些参数只有启用账号池或长期运行时才需要碰
3. 本地开发和 docs-site 的端口合同是什么

## 第一次部署，先决定这些

| 关注点 | 关键变量 | 默认值 | 什么时候该改 |
| --- | --- | --- | --- |
| 应用监听地址 | `HTTP_BIND` | 本地默认 `127.0.0.1:8080`；运行镜像默认 `0.0.0.0:8080` | 本地开发、容器部署或反向代理拓扑不同时 |
| 数据库存放位置 | `DATABASE_PATH` | `codex_vibe_monitor.db` | 想把 SQLite 和归档放到持久化卷时 |
| 上游代理目标 | `OPENAI_UPSTREAM_BASE_URL` | OpenAI 官方默认地址 | 你接的是自建兼容上游或其他转发层时 |
| 账号池写能力 | `UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET` | 无 | 只要你要新增账号、改账号或用 OAuth 登录，就必须配置 |
| 数据保留与归档 | `RETENTION_ENABLED`、`ARCHIVE_DIR`、各类 retention 天数 | 默认偏保守、默认不开启后台维护 | 想长期运行并控制主库体积时 |

如果你只是先把服务跑起来，通常先确认上表这 4 到 5 项就够了。

## 只想先跑起来，最小配置怎么理解

- 只想先接流量、看面板：`HTTP_BIND`、`DATABASE_PATH` 基本就会先决定。
- 不是转到默认 OpenAI 上游：再补 `OPENAI_UPSTREAM_BASE_URL`。
- 需要写入 Account Pool：必须提前准备 `UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET`。
- 需要长期跑并控制主库体积：再补 `RETENTION_ENABLED`、`ARCHIVE_DIR` 和各类 retention 窗口。

## 需要账号池写能力时，再看这些

- `UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET`：启用号池写入与加密落库的必填密钥
- `UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID`
- `UPSTREAM_ACCOUNTS_OAUTH_ISSUER`
- `UPSTREAM_ACCOUNTS_USAGE_BASE_URL`
- `UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS`
- `UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS`
- `UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS`
- `UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS`

如果你不打算启用 Account Pool 的写入能力，这一组可以后置。  
如果你准备让 OAuth 账号真正上线，不要把这组变量拖到最后才补。

## 需要长期运行时，尽早决定这些

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

这些参数控制在线明细、离线 archive 与后台 maintenance 行为。  
如果你希望数据库体积可控、raw 文件不无限增长、归档路径能备份，就不要只停在“默认值也能跑”这个阶段。

## 代理运行时常见的补充参数

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

## 开发与 docs-site 相关变量

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

## 继续阅读

- 想看单实例、反向代理、healthcheck 和持久化建议：看 [自部署](/deployment)
- 想看最常见的 readiness、无数据、账号池写失败问题：看 [排障](/troubleshooting)
- 想看仓库结构、核心命令与验收面：看 [开发](/development)
- 想看更深入的部署安全边界：回仓库读 [Deployment Guide](https://github.com/IvanLi-CN/codex-vibe-monitor/blob/main/docs/deployment.md)
