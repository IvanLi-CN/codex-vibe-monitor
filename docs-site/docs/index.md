---
title: Codex Vibe Monitor
description: 面向自部署与项目开发的 Codex Vibe Monitor 公共文档入口。
---

# Codex Vibe Monitor

Codex Vibe Monitor 是一套面向自部署的 OpenAI 兼容代理观测工作台。它把 `/v1/*` 调用捕获、实时与历史统计、SQLite 持久化、SSE 面板，以及上游账号池管理收在同一个项目里，适合想自己托管、自己排障、自己看数据的人。

## 这套项目适合谁

- 想部署一套自己的代理观测面板，先保证服务可用、数据可查、配置可控。
- 想排查请求失败、延迟抖动、成本变化或转发链路问题的运维与开发者。
- 想继续开发 UI、API、代理策略或账号池能力的协作者。

## 从哪里开始

### 我要先部署一套能用的实例

1. 先看 [快速开始](/quick-start) 里的“自部署路径”。
2. 再看 [配置参考](/config)，确认 `HTTP_BIND`、`DATABASE_PATH`、归档窗口和上游账号相关变量。
3. 需要网关、healthcheck 和安全边界时，继续读仓库里的 [Deployment Guide](https://github.com/IvanLi-CN/codex-vibe-monitor/blob/main/docs/deployment.md)。

### 我要先判断这个项目适不适合我

1. 看 [项目介绍](/product)，先理解它解决什么问题、核心页面是什么、为什么适合自部署。
2. 想看界面与组件证据，再去 [Storybook 导览](/storybook-guide.html)。

### 我要开发或扩展这个项目

1. 先看 [快速开始](/quick-start) 里的“开发路径”。
2. 再看 [项目介绍](/product) 理解页面地图和整体职责。
3. UI 细节与内部规范继续放在仓库的 [`docs/ui/`](https://github.com/IvanLi-CN/codex-vibe-monitor/tree/main/docs/ui)。

## 它解决什么问题

- 把代理调用、历史统计、实时事件和配置入口放在一套自部署应用里，而不是分散在脚本、日志和临时面板里。
- 用 SQLite 保留调用、账号、统计和归档数据，便于低门槛落地与备份。
- 让日常使用者先看 Dashboard 和 Records，运维问题再下钻到 Live、Stats 与 Settings。
- 在需要继续开发时，提供可运行的 React 前端、Rust 后端和 Storybook 证据面。

## 你会看到哪些核心界面

- `Dashboard`：先看今天的请求量、成本、成功率和更新时间。
- `Live`：看 forward proxy 节点的当前状态与短期表现。
- `Records`：按单次调用回放失败、耗时、代理落点和明细字段。
- `Stats`：看时间窗聚合、趋势和性能统计。
- `Settings`：管理价格目录、转发与系统级设置。
- `Account Pool`：维护 OAuth / API Key 上游账号、配额窗口与标签。

## 文档分工

- `docs-site/docs/` 负责 public docs：项目介绍、自部署入口、开发入口和 Storybook 导航。
- 仓库 `docs/deployment.md` 负责更深入的部署与安全边界。
- 仓库 `docs/ui/**` 负责 UI 规范与实现级真相源。
- Storybook 负责页面状态、组件证据和开发期复核，不替代生产运行态监控。
