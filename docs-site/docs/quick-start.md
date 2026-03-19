---
title: 快速开始
description: 先给自部署用户，再给开发者的 Codex Vibe Monitor 上手路径。
---

# 快速开始

先决定你要走哪条路径：

- 想先部署一套能用的实例：看“路径 A”。
- 想在本地开发、改前端或改后端：看“路径 B”。

## 路径 A：先部署一套可用实例

### 1. 准备最小运行参数

至少先确认这些变量：

- `DATABASE_PATH`：SQLite 主库路径
- `OPENAI_UPSTREAM_BASE_URL`：如果你不是转发到默认 OpenAI 上游，就需要显式设置
- `UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET`：如果要启用 Account Pool 的新增、更新或 OAuth 绑定，这是必填项
- `RETENTION_ENABLED` / `ARCHIVE_DIR`：如果你希望服务自动做冷热分层与离线归档，需要提前决定

### 2. 用容器跑起一个最小实例

```yaml
services:
  codex-vibe-monitor:
    image: ghcr.io/ivanli-cn/codex-vibe-monitor:latest
    restart: unless-stopped
    environment:
      HTTP_BIND: 0.0.0.0:8080
      DATABASE_PATH: /srv/app/data/codex_vibe_monitor.db
      UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET: change-me
    volumes:
      - ./data:/srv/app/data
```

如果你暂时只想看监控面板，不使用 Account Pool 的写能力，可以先不启用对应写入流程；但一旦需要新增账号、更新账号或走 OAuth 登录，就必须补上 `UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET`。

### 3. 把服务放在反向代理后面

- 推荐只对外暴露网关，不直接暴露应用监听端口。
- 容器内用 `HTTP_BIND=0.0.0.0:8080`，对外流量由 Traefik、Nginx 或其他反向代理转发。
- 更深入的安全边界与 healthcheck 约束，请读仓库里的 [Deployment Guide](https://github.com/IvanLi-CN/codex-vibe-monitor/blob/main/docs/deployment.md)。

### 4. 验证服务已经 ready

对应用自身做 readiness 检查：

```bash
curl -fsS http://127.0.0.1:8080/health
```

返回 `200 ok` 以后，再通过你的网关域名访问页面。

### 5. 下一步看哪里

- 需要细化运行参数：看 [配置参考](/config)。
- 需要先了解页面和能力边界：看 [项目介绍](/product)。
- 需要核对内部部署细节：继续读 [Deployment Guide](https://github.com/IvanLi-CN/codex-vibe-monitor/blob/main/docs/deployment.md)。

## 路径 B：本地开发或二次开发

### 环境要求

- Rust 工具链（仓库当前 CI 使用 `1.91.0`）
- Bun
- SQLite 开发库（Linux 上通常需要 `pkg-config` 与 `libsqlite3-dev`）

### 1. 安装仓库工具

```bash
bun install
```

### 2. 准备本地配置

在仓库根目录创建 `.env.local`，至少确认这些变量：

- `HTTP_BIND`（本地默认 `127.0.0.1:8080`）
- `DATABASE_PATH`
- `UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET`（需要测试号池写能力时）
- `VITE_BACKEND_PROXY`（如果前端代理目标不是默认后端地址）

### 3. 启动后端

```bash
cargo run
```

默认监听 `http://127.0.0.1:8080`。`GET /health` 在服务 ready 后返回 `200 ok`，否则返回 `503 starting`。

### 4. 启动前端

```bash
cd web
bun install
bun run dev -- --host 127.0.0.1 --port 60080
```

然后访问 `http://127.0.0.1:60080`。

### 5. 启动 Storybook

```bash
cd web
bun run storybook
```

默认访问地址是 `http://127.0.0.1:60082`。如需覆盖，可设置 `STORYBOOK_PORT`。

### 6. 启动 docs-site

```bash
cd docs-site
bun install
bun run dev
```

默认访问地址是 `http://127.0.0.1:60081`。如果同机已经启动 Storybook，本地 `storybook.html` 入口会跳到当前 Storybook dev server；如需覆盖目标 origin，可设置 `VITE_STORYBOOK_DEV_ORIGIN`。

### 7. 预览装配后的 Pages 站点（可选）

```bash
cd docs-site
bun run build
cd ../web
bun run storybook:build
cd ..
bash .github/scripts/assemble-pages-site.sh docs-site/doc_build web/storybook-static .tmp/pages-site
```

这一步会把 public docs 放在站点根目录，并把 Storybook 静态站嵌到 `.tmp/pages-site/storybook/`。
