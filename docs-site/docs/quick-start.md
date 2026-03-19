---
title: 快速开始
description: 在本地启动 Codex Vibe Monitor 的后端、前端、Storybook 与 docs-site。
---

# 快速开始

## 环境要求

- Rust 工具链（仓库当前 CI 使用 `1.91.0`）。
- Bun（前端、Storybook 与 docs-site 都依赖 Bun）。
- SQLite 开发库（Linux CI 使用 `pkg-config` 与 `libsqlite3-dev`）。

## 1. 安装根仓库工具

```bash
bun install
```

这一步会安装仓库级 Git tooling，并确保 `check:bun-first` 等校验脚本可用。

## 2. 准备配置

在仓库根目录创建 `.env.local`，至少确认这些变量：

- `HTTP_BIND`（默认 `127.0.0.1:8080`）
- `DATABASE_PATH`
- `UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET`（启用号池写入时必填）
- `VITE_BACKEND_PROXY`（若前端代理目标不是默认后端地址）

更多变量说明见 [配置参考](/config)。

## 3. 启动后端

```bash
cargo run
```

后端默认监听 `http://127.0.0.1:8080`。`GET /health` 在服务 ready 后返回 `200 ok`，否则返回 `503 starting`。

## 4. 启动前端

```bash
cd web
bun install
bun run dev -- --host 127.0.0.1 --port 60080
```

然后访问 `http://127.0.0.1:60080`。

## 5. 启动 Storybook

```bash
cd web
bun run storybook
```

默认访问地址是 `http://127.0.0.1:60082`。如需覆盖，可设置 `STORYBOOK_PORT`。

## 6. 启动 docs-site

```bash
cd docs-site
bun install
bun run dev
```

默认访问地址是 `http://127.0.0.1:60081`。如果同时启动了 Storybook，本页里的 Storybook 入口会跳到本地 Storybook dev server；如需覆盖目标 origin，可设置 `VITE_STORYBOOK_DEV_ORIGIN`。

## 7. 预览装配后的 Pages 站点（可选）

```bash
cd docs-site
bun run build
cd ../web
bun run storybook:build
cd ..
bash .github/scripts/assemble-pages-site.sh docs-site/doc_build web/storybook-static .tmp/pages-site
```

这一步会把 public docs 放在站点根目录，并把 Storybook 静态站嵌到 `.tmp/pages-site/storybook/`。
