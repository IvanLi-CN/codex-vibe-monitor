---
title: 自部署
description: 面向长期运行实例的 Codex Vibe Monitor 自部署指南。
---

# 自部署

这页默认你已经不是“先试试看”，而是准备把服务长期跑起来。

## 推荐拓扑

- 对外只暴露网关，不直接暴露应用监听端口。
- 应用服务留在内网或容器网络里。
- 浏览器、脚本或现有兼容客户端都从你的网关入口进入。

如果你只是想先验证镜像能不能跑，用 [快速开始](/quick-start) 即可；  
这页更关心长期运行、持久化和上线口径。

## 生产基线 Compose

```yaml
services:
  codex-vibe-monitor:
    image: ghcr.io/ivanli-cn/codex-vibe-monitor:latest
    restart: unless-stopped
    environment:
      HTTP_BIND: 0.0.0.0:8080
      DATABASE_PATH: /srv/app/data/codex_vibe_monitor.db
    volumes:
      - ./data:/srv/app/data
    healthcheck:
      test: ["CMD", "curl", "-fsS", "http://127.0.0.1:8080/health"]
      interval: 15s
      timeout: 5s
      retries: 6
      start_period: 60s
```

这份基线示例的目标不是“最短”，而是让你一开始就带上持久化和 readiness。  
如果你还需要 Account Pool 写能力，再补 `UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET`。

## 长期运行前，最先决定这些变量

| 变量 | 作用 | 什么时候必须配 |
| --- | --- | --- |
| `HTTP_BIND` | 服务监听地址 | 容器部署或网关拓扑不同的时候 |
| `DATABASE_PATH` | SQLite 主库路径 | 想把数据库放在持久化卷时 |
| `OPENAI_UPSTREAM_BASE_URL` | OpenAI 兼容上游地址 | 不是转发到默认 OpenAI 上游时 |
| `UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET` | Account Pool 写入与 OAuth 绑定密钥 | 需要账号池写能力时 |
| `RETENTION_ENABLED` / `ARCHIVE_DIR` | 后台归档与离线目录 | 想长期运行并控制主库体积时 |

## 网关与暴露面

- 推荐只对外暴露网关，不直接暴露应用监听端口。
- 如果走容器部署，应用内通常用 `HTTP_BIND=0.0.0.0:8080`，但对外流量仍然应该由 Traefik、Nginx 或其他反向代理承接。
- `X-Forwarded-*` 这类头只应该由受信任网关产生；不要把应用端口直接开放到公网后再指望这些头有安全意义。

## Readiness 与健康检查

应用的 `GET /health` 表示 readiness，而不是“进程活着”：

- 服务完成核心初始化并开始监听后返回 `200 ok`
- 在此之前返回 `503 starting`

典型 healthcheck 口径：

```yaml
healthcheck:
  test: ["CMD", "curl", "-fsS", "http://127.0.0.1:8080/health"]
  interval: 15s
  timeout: 5s
  retries: 6
  start_period: 60s
```

如果你的网关或编排系统会在服务还没 ready 时就导流，问题通常不会表现成“完全打不开”，而是表现成间歇性失败、启动窗口大量错误或首批请求异常。

## 持久化、归档与备份

- `DATABASE_PATH` 决定主库位置，建议直接挂载到持久化卷。
- `ARCHIVE_DIR` 与 `PROXY_RAW_DIR` 使用相对路径时，会锚定到 `DATABASE_PATH` 同级目录。
- 如果你开启 retention / archive，备份时不要只看主库，还要把 archive 目录一起纳入。
- 镜像本身是无状态的，真正需要你保住的是 SQLite 与相关落盘目录。

## Account Pool 与 OAuth 部署备注

- 只读查看账号列表不代表账号池写能力已经就绪。
- 只要涉及新增账号、更新账号、删除账号或 OAuth 绑定，`UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET` 就必须存在。
- OAuth callback 固定走 `/api/pool/upstream-accounts/oauth/callback`，反向代理要正确透传实际请求的 `Origin/Host`。
- 如果同步或路由日志出现 `failed to contact oauth codex upstream`，优先排查出网连通性，而不是先怀疑前端页面。

## 上线前检查

- `curl http://127.0.0.1:8080/health` 已经返回 `200 ok`
- 通过你的网关域名或内网入口能正常打开页面
- 至少已有一批真实调用流量进入，Dashboard / Records 已经能看到数据
- 如果要用 Account Pool 写能力，`UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET` 已配置
- 数据库目录与归档目录已经明确挂载，不会因为容器重启而丢失

## 继续阅读

- 想看最短启动路径：看 [快速开始](/quick-start)
- 想看首次部署真正要决定的变量：看 [配置与运行](/config)
- 想先处理 readiness、无数据或账号池写失败问题：看 [排障](/troubleshooting)
- 想看更完整的部署安全边界：回仓库读 [Deployment Guide](https://github.com/IvanLi-CN/codex-vibe-monitor/blob/main/docs/deployment.md)
