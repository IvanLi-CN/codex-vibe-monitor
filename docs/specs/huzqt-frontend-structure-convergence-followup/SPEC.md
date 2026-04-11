# 前端结构收敛 follow-up（#huzqt）

## 状态

- Status: 已验证
- Created: 2026-04-11
- Last: 2026-04-11

## 背景 / 问题陈述

- `web/src/pages/account-pool/` 仍集中承载超大页面实现，`UpstreamAccounts.page-local-shared.tsx`、`UpstreamAccounts.page-impl.tsx`、`UpstreamAccountCreate.sections.tsx`、`UpstreamAccountCreate.page-impl.tsx` 等文件同时混合 controller、section 组装、overlay/drawer、draft/state 与交互 glue，导航与 review 成本过高。
- 该区域还保留多处生产级 `@ts-nocheck`，类型边界与职责边界一起失焦，后续继续叠加功能会持续放大回归风险。
- 前端数据层也仍有中大型热点文件（尤其 account-pool 相关 API / hook / story runtime），需要和页面收敛一起建立更稳定的模块边界。

## 目标 / 非目标

### Goals

- 把 account-pool 页面族拆回可 review 的职责粒度，建立稳定的 page shell / controller / section / shared model 边界。
- 优先消除本轮触及的生产文件中的 `@ts-nocheck`，至少不再让核心 page entry 继续依赖它。
- 保持现有路由、默认导出、查询参数、Storybook 入口名称、页面行为与 API 调用契约不变。
- 为共享测试机上的浏览器级 smoke 提供可复用的前端实际环境验证脚本。

### Non-goals

- 不重写 UI 视觉设计，不新增产品功能。
- 不为了“压行数”去重排 `translations.ts`、纯 story fixture 或无关页面。
- 不修改后端 schema、HTTP/SSE 契约或账号池业务语义。

## 范围（Scope）

### In scope

- `web/src/pages/account-pool/UpstreamAccountCreate*` 的页面组装层收敛，拆出 primary card、dialogs 与四个 tab section。
- 为 account-pool create/list 页面补充浏览器级 smoke，覆盖真实运行时下的 `oauth / batchOauth / import / apiKey` 页面入口。
- 对应 Storybook 入口复用既有稳定 stories，产出新的本地视觉证据。
- shared-testbox 前端 smoke 与容器化 runtime 验证记录。

### Out of scope

- 无关 dashboard / records / live 页面结构收敛。
- i18n 文案内容本身的新增或整理。
- Playwright 全量体系重写。

## 实施结果

- `web/src/pages/account-pool/UpstreamAccountCreate.sections.tsx` 从巨型 section 装配文件收敛为 page shell，仅保留 heading / alerts / tab 切换与主卡片挂载。
- 新增 `UpstreamAccountCreate.primary-card.tsx` 与 `UpstreamAccountCreate.dialogs.tsx`，把卡片 header、批量默认 metadata 工具区、导入校验弹窗、分组设置弹窗、重复详情弹窗从 page shell 剥离。
- 新增四个独立 tab section 文件：
  - `web/src/pages/account-pool/UpstreamAccountCreate.oauth-section.tsx`
  - `web/src/pages/account-pool/UpstreamAccountCreate.batch-oauth-section.tsx`
  - `web/src/pages/account-pool/UpstreamAccountCreate.import-section.tsx`
  - `web/src/pages/account-pool/UpstreamAccountCreate.api-key-section.tsx`
- 本轮新增/重写的 page shell 与 section 组装文件不再使用 `@ts-nocheck`；既有更深层 controller / shared / action 热点保留到后续轮次继续收敛。
- 新增 `web/tests/e2e/account-pool-create-smoke.spec.ts`，用浏览器级 smoke 固化 list + 四种 create mode 的最低可用性检查。

## 验证记录

- `cd web && bun run lint`（通过；仓库既有 `react-hooks/exhaustive-deps` warnings 保持原状）
- `cd web && bun run test`（82 files / 783 tests 通过）
- `cd web && bun run build`（通过）
- `cd web && bun run build-storybook`（通过）
- 本地浏览器 smoke：`E2E_BASE_URL=http://127.0.0.1:30031 bun run test:e2e tests/e2e/account-pool-create-smoke.spec.ts`（5/5 通过）
- shared-testbox 容器化 runtime smoke：
  - `/srv/codex/workspaces/ivan/codex-vibe-monitor__4dd0653c/runs/frontend_20260411_144440_bcddf0d9`
  - `/srv/codex/workspaces/ivan/codex-vibe-monitor__4dd0653c/runs/frontend_final_retry_20260411_150718_bcddf0d9`
  - 通过远端 Docker runtime + 本地 SSH 隧道执行 `bun run test:e2e tests/e2e/account-pool-create-smoke.spec.ts`，两轮均 5/5 通过。

## Visual Evidence

- 视觉证据基于既有 Storybook 稳定 stories 重新生成，本地已回传：
  - `Account Pool/Pages/Upstream Account Create/Overview -> Default`
  - `Account Pool/Pages/Upstream Account Create/Batch OAuth -> Ready`
  - `Account Pool/Pages/Upstream Account Create/API Key -> Name Conflict`
- 本地证据文件位于 `.codex/artifacts/frontend-visuals/`，不进入本次提交物。
