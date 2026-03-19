---
title: 排障
description: 自部署与开发时最常见的问题、症状与排查入口。
---

# 排障

这页只收最常见、最容易卡住自部署和开发流程的问题。更深的运行时细节与安全边界，继续回仓库读 [Deployment Guide](https://github.com/IvanLi-CN/codex-vibe-monitor/blob/main/docs/deployment.md)。

## `/health` 一直返回 `503 starting`

这表示服务还没有完成 readiness，而不是单纯“进程挂了”。

- 服务在核心初始化完成并开始监听后，才会返回 `200 ok`。
- 如果你前面挂了网关或容器编排，应该让流量等待健康检查通过后再导入。
- 如果长时间不切到 `200 ok`，优先看服务日志，确认数据库路径、schema 初始化和启动期任务是否异常。

## 页面能打开，但 Dashboard / Records 里没有数据

这通常不是前端坏了，而是流量还没真正经过这套服务。

- 先确认你的 OpenAI 兼容客户端、脚本或网关已经把 `/v1/*` 请求指向 Codex Vibe Monitor。
- `GET /health` 返回 `200 ok` 只表示服务 ready，不表示已经有业务流量进来。
- 用一条真实请求打进来后，再看 Dashboard、Live、Records 是否开始出现数据。

## Account Pool 页面能看，但新增、更新或 OAuth 绑定失败

最常见的原因是没有配置 `UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET`。

- 没有这个密钥时，账号列表通常仍可读。
- 但新增账号、更新账号、删除账号和 OAuth 绑定都会被拒绝。
- 如果你准备长期使用 Account Pool，把这个密钥视为部署前置项，而不是等页面报错后再补。

## OAuth callback 或账号同步异常

这类问题优先看反向代理和出网链路。

- OAuth callback 固定走 `/api/pool/upstream-accounts/oauth/callback`。
- 服务会根据实际请求的 `Origin/Host` 生成 redirect URI，所以反向代理必须正确透传这些头。
- 如果错误信息包含 `failed to contact oauth codex upstream`，优先排查服务到 `chatgpt.com/backend-api/codex` 的出网连通性。

## 磁盘持续增长，不知道该从哪里收口

默认情况下，retention / archive 不是自动开启的。

- 长期运行前先决定 `DATABASE_PATH`、`ARCHIVE_DIR` 和 retention 窗口。
- 如果只备份主库，不备份 archive 目录，后续做冷热分层后数据链路会不完整。
- 相对路径的 `ARCHIVE_DIR` 与 `PROXY_RAW_DIR` 会锚定到 `DATABASE_PATH` 同级目录，不要想当然按当前工作目录去找。

## 失败记录里的 `failureKind` 应该怎么理解

下面这几个最常见：

| failureKind | 典型含义 |
| --- | --- |
| `request_body_read_timeout` | 客户端上传过慢，或者前置代理链路在读请求体阶段阻塞 |
| `request_body_stream_error_client_closed` | 客户端在上传阶段主动断开 |
| `failed_contact_upstream` | 服务到上游连接失败 |
| `upstream_handshake_timeout` | 上游在握手或首响应阶段太慢 |
| `upstream_stream_error` | 上游开始返回后又在流式阶段中途失败 |

如果你是在生产环境里追这类问题，优先结合 Records 明细、Stats 趋势和网关日志一起看，不要只盯单个错误字符串。

## 第一次上线前，至少确认这些

- `GET /health` 已经稳定返回 `200 ok`
- 页面可以通过你的网关入口访问
- 至少有一批真实请求已经经过这套服务
- 如果要用 Account Pool 写能力，`UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET` 已配置
- 数据库目录与归档目录已经挂到持久化位置

## 继续阅读

- 想按最短路径先跑起来：看 [快速开始](/quick-start)
- 想按场景判断参数：看 [配置与运行](/config)
- 想按长期运行口径部署：看 [自部署](/deployment)
