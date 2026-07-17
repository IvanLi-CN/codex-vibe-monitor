# Installable PWA 运行时与 Dashboard 概览离线快照 - Implementation

## Current State

- Canonical spec: `docs/specs/m9p2w-installable-pwa-runtime/SPEC.md`
- Implementation summary:
  - installable-runtime PWA 仍由 `vite-plugin-pwa` `injectManifest`、manifest、service worker、install control、Safari manual guidance、prompt-style update 与 offline shell banner 组成。
  - Dashboard 概览离线数据改为应用层 IndexedDB snapshot store：五个固定 range 各保存最近一份成功快照，不把 `/api/*` 缓存职责塞进 service worker。
  - `DashboardActivityOverview` 已接入 `live` / `cached-offline` / `not-cached-yet` 三态；`working conversations` 明确保留在线依赖，并在离线重开时显示不可用语义。

## 状态

- Status: 已实现
- Created: 2026-07-17
- Last: 2026-07-17

## 实现范围

### Runtime contract

- base-aware `site.webmanifest`
- service worker inject-manifest build
- browser-native install prompt + Safari manual guidance
- waiting-update prompt
- offline shell banner

### Dashboard overview snapshots

- IndexedDB schema `cvm-dashboard-overview-snapshots`
- 五个固定 range 的最新成功快照写入 / 读取 / schema version 校验
- 在线首次渲染后顺序预热剩余 range
- 离线 / 网络类失败时的 cached fallback
- `not-cached-yet` 空状态与 `cachedAt` banner

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cd web && bun run test`
- `cd web && bun run test-storybook`
- `cd web && bun run test:e2e:pwa`
- `cd web && bun run build`

## 文档更新（Docs Updated）

- `docs/specs/m9p2w-installable-pwa-runtime/SPEC.md`
- `docs/specs/m9p2w-installable-pwa-runtime/IMPLEMENTATION.md`
- `docs/specs/m9p2w-installable-pwa-runtime/HISTORY.md`
- `docs/specs/README.md`

## 关键实现点

- `web/src/features/dashboard/dashboardOverviewSnapshots.ts`
  - 定义 snapshot schema、range query matrix、IndexedDB 读写与最新快照覆盖策略。
- `web/src/hooks/useDashboardOverviewSnapshotRuntime.ts`
  - 处理在线预热、离线读取、网络类失败 fallback、reconnect refresh 与 ready range 状态。
- `web/src/features/dashboard/DashboardActivityOverview.tsx`
  - 在根概览内切换 live / cached / empty 三态，并把 snapshot bundle 注入今天概览、24h/7d 面板、heatmap、usage calendar。
- `web/src/features/dashboard/DashboardWorkingConversationsSection.tsx`
  - 保持在线依赖，但在离线重开且无 SSE snapshot 时明确显示 unavailable 语义。
- `web/tests/pwa/installable-runtime.spec.ts`
  - 覆盖 install prompt、waiting update、offline shell，以及五个 range 的 overview snapshot 离线切换与重连恢复。

## Visual Evidence

- Canonical owner-facing captures live in `docs/specs/m9p2w-installable-pwa-runtime/SPEC.md#visual-evidence`.
- Captured artifacts:
  - `./assets/pwa-install-prompt-desktop.png`
  - `./assets/pwa-safari-manual-desktop.png`
  - `./assets/pwa-update-banner-desktop.png`
  - `./assets/pwa-offline-banner-desktop.png`
  - `./assets/pwa-dashboard-offline-cached-today.png`
  - `./assets/pwa-dashboard-offline-cached-history.png`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: installable-runtime PWA build/runtime contract 落地
- [x] M2: app shell install/update/offline UX 落地
- [x] M3: Dashboard overview IndexedDB snapshots 与 offline fallback 落地
- [x] M4: PWA / Storybook / Vitest 验证面通过
- [x] M5: `#m9p2w` spec current truth 同步完成
