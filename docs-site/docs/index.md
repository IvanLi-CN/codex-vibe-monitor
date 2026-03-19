---
title: Codex Vibe Monitor 文档首页
description: Codex Vibe Monitor 公共文档入口。
---

# Codex Vibe Monitor 文档

Codex Vibe Monitor 用于捕获 OpenAI 兼容 `/v1/*` 代理调用，写入 SQLite，并通过 REST API、SSE 与前端仪表盘提供实时与历史可观测性。

## 先看这 4 件事

1. 第一次跑项目，先看 [快速开始](/quick-start)。
2. 需要准备 `.env.local` 或确认端口与部署基路径，直接跳到 [配置参考](/config)。
3. 想快速理解 Dashboard、Live、Records、Settings 与 Account Pool 的分工，看 [产品说明](/product)。
4. 想预览页面状态与组件证据，打开 [Storybook](/storybook.html) 或 [Storybook 导览](/storybook-guide.html)。

## 文档导航

- [快速开始](/quick-start)
- [配置参考](/config)
- [产品说明](/product)
- [Storybook 导览](/storybook-guide.html)
- [GitHub 仓库](https://github.com/IvanLi-CN/codex-vibe-monitor)

## 适合谁看

- 新接手仓库的开发者：先用本文档梳理本地启动、配置与页面地图。
- 需要核对 UI 状态的评审者：直接从 Storybook 导览进入核心 stories。
- 需要判断部署边界的协作者：先读配置参考和产品说明，再回到仓库里的部署与 UI 规范细节。

## 当前首版范围

- 中文单语 public docs。
- 面向开发者与评审协作者的最小入口集合。
- Storybook 与文档站装配为同一个 GitHub Pages 站点，但更深入的内部规范仍保留在仓库 `docs/**`。
