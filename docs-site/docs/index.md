---
title: Codex Vibe Monitor
description: 面向自部署与项目开发的 Codex Vibe Monitor 公共文档入口。
---

# Codex Vibe Monitor

Codex Vibe Monitor 是一套面向自部署的 OpenAI 兼容代理观测工作台。它把 `/v1/*` 调用捕获、实时与历史统计、SQLite 持久化、SSE 面板，以及上游账号池管理收在同一个项目里，目标不是“再做一个图表页”，而是让你在自己的环境里看得到、查得到、调得动。

## 这套项目解决什么问题

它不是只有图表的只读 dashboard，而是同时交付四个稳定的交付面：

- 代理入口：捕获 OpenAI 兼容 `/v1/*` 请求与响应，统一承接上游流量。
- 观测面板：Dashboard、Live、Records、Stats 和 Settings 负责让你看到实时与历史状态。
- 账号池能力：统一管理 OAuth / API Key 上游账号、同步状态、配额窗口与标签。
- 开发与验收面：公开 docs-site 负责使用说明，Storybook 负责页面与组件证据。

## 谁应该从这里开始

- 想自部署一套自己的代理观测台，而不是把数据交给第三方 SaaS 的人。
- 想排查失败调用、上游抖动、延迟问题、成本变化或账号池状态的人。
- 想继续改后端、前端、Storybook 或文档链路的协作者。

## 三条最短阅读路径

### 我要先部署一套能用的实例

1. 先看 [快速开始](/quick-start)，选“自部署单实例”路径。
2. 再看 [配置与运行](/config)，确认 `HTTP_BIND`、`DATABASE_PATH`、归档窗口和账号池相关变量。
3. 需要长期运行、挂反向代理或做上线前检查时，看 [自部署](/deployment)。
4. 服务能打开但没数据、账号池写不进去或磁盘持续膨胀时，看 [排障](/troubleshooting)。

### 我要先判断这个项目适不适合我

1. 看 [项目介绍](/product)，先理解它解决什么问题、核心页面是什么、为什么适合自部署。
2. 想看界面与组件证据，再去 [Storybook 导览](/storybook-guide.html)。

### 我要开发或扩展这个项目

1. 先看 [开发](/development) 了解仓库结构、核心命令和验收面。
2. 再看 [项目介绍](/product) 理解页面职责和产品边界。
3. UI 细节与内部规范继续放在仓库的 [`docs/ui/`](https://github.com/IvanLi-CN/codex-vibe-monitor/tree/main/docs/ui)。

## 自部署前先知道这四件事

- `GET /health` 是 readiness，不是“进程活着”探针；初始化未完成时会返回 `503 starting`。
- 生产建议只暴露网关，不要把应用监听端口直接暴露到公网。
- 如果你要新增账号、更新账号或使用 OAuth 账号池，`UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET` 不是可选项。
- retention / archive 默认不是自动帮你开好的；长期运行前要先决定 `DATABASE_PATH`、`ARCHIVE_DIR` 和保留窗口。

## 文档地图

1. [项目介绍](/product)：用来判断这项目适不适合你，而不是直接教你部署。
2. [快速开始](/quick-start)：最短启动路径，优先解决“先跑起来”。
3. [配置与运行](/config)：按场景梳理第一次部署、账号池写能力和长期运行参数。
4. [自部署](/deployment)：生产拓扑、Compose、网关、持久化与上线检查。
5. [排障](/troubleshooting)：把最常见的“能打开但不能用”问题集中收口。
6. [开发](/development)：给改代码的人看仓库结构、命令和验收面。
7. [Storybook 导览](/storybook-guide.html)：页面与组件证据入口。

## 文档分工

- `docs-site/docs/` 负责 public docs：项目介绍、自部署入口、开发入口和 Storybook 导航。
- 仓库 `docs/deployment.md` 负责更深入的部署与安全边界。
- 仓库 `docs/ui/**` 负责 UI 规范与实现级真相源。
- Storybook 负责页面状态、组件证据和开发期复核，不替代生产运行态监控。
