---
title: 产品说明
description: Codex Vibe Monitor 的页面地图、核心数据流与公开文档边界。
---

# 产品说明

本文档帮助协作者快速理解 Codex Vibe Monitor 的主要页面、数据边界与适合使用 Storybook 的场景。

## 核心体验

Codex Vibe Monitor 的目标不是替代上游控制台，而是把代理调用、统计、实时流与号池状态整理成可观测工作台：

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

## 何时优先看 Storybook

- 想确认页面状态、筛选器、表格与卡片在 mock 数据下的表现时
- 想快速核对 Dashboard、InvocationTable、RecordsPage、SettingsPage 或 Account Pool 页面边界时
- 想在不启动完整后端的前提下做页面或组件复核时

## 非目标

- 不在 public docs 里完整覆盖所有运维排障流程
- 不把 specs / plan 目录暴露成公开文档导航
- 不把 Storybook 当成生产运行态的真实监控来源
