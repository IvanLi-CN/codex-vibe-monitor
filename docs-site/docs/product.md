---
title: 项目介绍
description: Codex Vibe Monitor 的定位、核心能力、页面地图与自部署价值。
---

# 项目介绍

Codex Vibe Monitor 不是一个只看总量的静态 dashboard。它把 OpenAI 兼容代理的调用捕获、实时事件、历史统计、配置入口和上游账号池放在同一套自部署应用里，目标是让你在自己的环境中看得到、查得到、调得动。

## 这套项目提供什么

- 一套适合自部署的观测工作台：Rust 后端、React 前端、SQLite 持久化，单项目即可落地。
- 从实时到历史的连续视图：SSE 推送、聚合统计、调用明细和趋势页面放在一起。
- 面向运维和开发的统一入口：不仅能看，还能继续排查、调配置、核对 UI。
- 可继续扩展的工程骨架：Storybook、内部 UI 文档和 repo docs 都保留在仓库里。

## 为什么它适合自部署

- 数据留在你自己的环境里，SQLite 与归档策略也由你自己决定。
- 部署模型直接，容器镜像即可运行，不要求拆成多套服务才能看到面板。
- 生产推荐只暴露网关，对外安全边界清楚，服务自身的 readiness 和 healthcheck 也有明确约束。
- 号池能力已经内联在主服务里，OAuth 数据面不再依赖额外 sidecar。

## 核心能力地图

Codex Vibe Monitor 的重点不是替代上游控制台，而是把代理调用、统计、实时流与号池状态整理成一个面向排障与运营的工作台：

- Dashboard：聚合当天关键指标、趋势与摘要卡片
- Live：观察 forward proxy 节点的当前状态与短期表现
- Records：回放调用记录、筛选条件与失败明细
- Stats：查看时间窗汇总、成功/失败趋势与性能统计
- Settings：维护价格、代理、转发与系统级配置
- Account Pool：管理 OAuth/API Key 上游账号、路由与标签

## 页面结构

### Dashboard

- 承担总览入口
- 适合先确认今天的请求量、成本、成功率与更新时间

### Live

- 聚焦实时代理节点状态
- 适合核对当前 proxy 节点是否健康、是否有异常波动

### Records

- 聚焦单次调用与筛选条件
- 适合排查失败、查看 prompt cache key、IP、耗时与代理落点

### Stats

- 聚焦时间窗统计与趋势图
- 适合做更长时间范围的成功/失败、耗时与分桶分析

### Settings

- 聚焦价格、代理与运维配置入口
- 适合维护转发策略、模型价格目录与代理节点来源

### Account Pool

- `Upstream Accounts`：查看账号列表、配额窗口与同步状态
- `Upstream Account Create`：创建 OAuth / API Key 账号
- `Tags`：维护标签与路由语义

## 数据与部署边界

- 后端使用 SQLite 持久化调用、统计、号池与配置数据。
- 前端通过 REST API 与 SSE 拉取实时与历史视图。
- 生产部署建议只暴露网关，不直接暴露应用监听端口。
- 公开 docs-site 只承担入口和导航，不替代仓库内部的 `docs/specs/**`、`docs/ui/**` 与部署排障文档。

## 如果你是开发者，下一步通常看哪里

- 想改页面、状态和组件表现：优先看 [Storybook 导览](/storybook-guide.html)。
- 想核对更深的部署与安全假设：回仓库读 [Deployment Guide](https://github.com/IvanLi-CN/codex-vibe-monitor/blob/main/docs/deployment.md)。
- 想看 UI 规范和内部事实来源：回仓库读 [`docs/ui/`](https://github.com/IvanLi-CN/codex-vibe-monitor/tree/main/docs/ui)。

## 何时优先看 Storybook

- 想确认页面状态、筛选器、表格与卡片在 mock 数据下的表现时
- 想快速核对 Dashboard、InvocationTable、RecordsPage、SettingsPage 或 Account Pool 页面边界时
- 想在不启动完整后端的前提下做页面或组件复核时

## 非目标

- 不在 public docs 里完整覆盖所有运维排障流程
- 不把 specs / plan 目录暴露成公开文档导航
- 不把 Storybook 当成生产运行态的真实监控来源
