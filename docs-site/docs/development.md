---
title: 开发
description: Codex Vibe Monitor 的仓库结构、本地命令与验收面。
---

# 开发

## 先认仓库结构

- `src/`：Rust 后端、调度、HTTP API、SSE 与 SQLite 持久化
- `web/`：React + Vite 应用、页面、hooks 与 Storybook
- `docs-site/`：公开 Rspress 文档站
- `docs/`：内部部署文档、UI 规范、规格与历史计划

## 你要改什么，就先去哪

- 想改代理、接口、SSE 或持久化：从 `src/` 开始。
- 想改页面、图表、表格和状态流：从 `web/src/` 开始。
- 想改公开文档入口：从 `docs-site/docs/` 开始。
- 想改 UI 规范、视觉真相源与内部设计约束：回仓库看 `docs/ui/` 与 Storybook stories。

## 本地服务合同

- Backend：`127.0.0.1:8080`
- App dev：`127.0.0.1:60080`
- docs-site：`127.0.0.1:60081`
- Storybook：`127.0.0.1:60082`

这些端口都允许覆盖，但仓库脚本、文档和 CI 默认按这套口径组织。

## 核心命令

### 后端

```bash
cargo fmt
cargo check
cargo test
cargo run
```

### 前端

```bash
cd web
bun install
bun run lint
bun run dev -- --host 127.0.0.1 --port 60080
bun run test
bun run build
bun run storybook
bun run storybook:build -- --quiet
```

### docs-site

```bash
cd docs-site
bun install
bun run dev
bun run build
```

## 验收面

- 运行时应用：后端 + Vite dev server，用来验证真实 API、SSE 和状态联动
- Storybook：页面、组件和 mock 数据下的 UI 验收面
- docs-site：面向自部署用户与开发协作者的 public docs

## 最常见的开发路径

1. `cargo run` 启动后端
2. `cd web && bun run dev -- --host 127.0.0.1 --port 60080` 启动前端
3. 如果在改页面或组件，再额外启动 `cd web && bun run storybook`
4. 如果在改 public docs，再额外启动 `cd docs-site && bun run dev`

## CI 与发布面

- `CI PR` / `CI Main` 会覆盖前后端检查、构建产物和文档链路 smoke
- `Docs Pages` 负责 docs-site + Storybook 的组装与 GitHub Pages 发布
- `Release` 负责容器镜像与版本发布

## 继续阅读

- 想先把服务跑起来：看 [快速开始](/quick-start)
- 想按长期运行口径部署：看 [自部署](/deployment)
- 想看最常见的运行问题：看 [排障](/troubleshooting)
- 想先理解页面职责和产品边界：看 [项目介绍](/product)
