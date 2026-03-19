---
title: 快速开始
description: 先给自部署用户，再给开发者的 Codex Vibe Monitor 上手路径。
---

# 快速开始

这页只解决一件事：先把项目跑起来，然后确认它真的能用。

## 先选一种路径

- 路径 A，自部署单实例：优先给想直接跑镜像的人。
- 路径 B，本地开发：优先给要改后端、前端、Storybook 或 docs-site 的人。

## 前置依赖

- Docker（如果走路径 A，自部署单实例）
- Rust 工具链（仓库当前 CI 使用 `1.91.0`）
- Bun
- SQLite 开发库（Linux 上通常需要 `pkg-config` 与 `libsqlite3-dev`）

## 路径 A：先部署一套能看的实例

### 1. 先准备一个持久化目录

```bash
mkdir -p data
```

### 2. 直接拉镜像跑起来

```bash
docker run -d \
  --name codex-vibe-monitor \
  -p 8080:8080 \
  -v "$(pwd)/data:/srv/app/data" \
  ghcr.io/ivanli-cn/codex-vibe-monitor:latest
```

这条命令适合先验证镜像、页面和基础观测链路。  
如果你接下来还要新增账号、更新账号或使用 OAuth 账号池，再补 `UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET`。

### 3. 确认服务已经 ready

```bash
curl -fsS http://127.0.0.1:8080/health
```

返回 `200 ok` 以后，服务才算 ready。  
如果还是 `503 starting`，先不要让网关把流量导进去。

### 4. 让第一批真实流量走进来

- 把你现有的 OpenAI 兼容客户端、脚本或网关流量指向这套服务。
- 然后确认 Dashboard、Live、Records 至少有一页开始出现数据。
- 如果页面能打开但没有任何调用记录，说明服务活着了，但还没有真正接入。

### 5. 什么时候算“第一阶段完成”

- `/health` 已返回 `200 ok`
- 页面能打开
- 至少有一批真实调用已经被捕获

### 6. 跑通以后下一步去哪里

- 想按场景梳理参数：看 [配置与运行](/config)
- 想按长期运行口径部署：看 [自部署](/deployment)
- 想先了解页面和能力边界：看 [项目介绍](/product)
- 卡在 readiness、没有数据、账号池写失败：看 [排障](/troubleshooting)

## 路径 B：本地开发

### 1. 安装仓库工具

```bash
bun install
```

这一步会安装仓库级工具，并确保 `check:bun-first` 等检查脚本可用。

### 2. 准备本地配置

在仓库根目录创建 `.env.local`，至少确认这些变量：

- `HTTP_BIND`（本地默认 `127.0.0.1:8080`）
- `DATABASE_PATH`
- `UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET`（要测试账号池写能力时）
- `VITE_BACKEND_PROXY`（如果前端代理目标不是默认后端地址）

### 3. 启动后端

```bash
cargo run
```

默认监听 `http://127.0.0.1:8080`。`GET /health` ready 后返回 `200 ok`，否则返回 `503 starting`。

### 4. 启动前端

```bash
cd web
bun install
bun run dev -- --host 127.0.0.1 --port 60080
```

访问 `http://127.0.0.1:60080`。

### 5. 按需要补 Storybook 与 docs-site

```bash
cd web
bun run storybook
```

默认访问 `http://127.0.0.1:60082`。

```bash
cd docs-site
bun install
bun run dev
```

默认访问 `http://127.0.0.1:60081`。如果同机已经启动 Storybook，本地 `storybook.html` 会跳转到当前 Storybook dev server。

### 6. 需要看最终静态发布面时，再组装 Pages

```bash
cd docs-site
bun run build
cd ../web
bun run storybook:build
cd ..
bash .github/scripts/assemble-pages-site.sh docs-site/doc_build web/storybook-static .tmp/pages-site
```

这一步会把 public docs 放在站点根目录，并把 Storybook 静态站嵌到 `.tmp/pages-site/storybook/`。

## 下一步

- 想按长期运行口径部署，而不是只本地跑通：看 [自部署](/deployment)
- 想知道第一次部署应该真正决定哪些参数：看 [配置与运行](/config)
- 想开始改代码：看 [开发](/development)
- 想先核对页面和组件状态：看 [Storybook](/storybook.html)
- 卡在“服务能开但不好用”：看 [排障](/troubleshooting)
