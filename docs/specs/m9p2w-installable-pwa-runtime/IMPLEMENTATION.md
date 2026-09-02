# Installable PWA 运行时与 Dashboard 概览离线快照 - Implementation

## Current State

- Canonical spec: `docs/specs/m9p2w-installable-pwa-runtime/SPEC.md`
- Implementation summary:
  - installable-runtime PWA 仍由 `vite-plugin-pwa` `injectManifest`、manifest、service worker、install control、Safari manual guidance、prompt-style update 与 offline shell banner 组成。
  - install control 不再在头栏暴露常驻 button；当浏览器满足安装条件时，app shell 会自动弹出 install prompt / manual guidance，并保持窄屏居中 modal 语义。
  - Dashboard 概览离线数据改为应用层 IndexedDB snapshot store：五个固定 range 各保存最近一份成功快照，不把 `/api/*` 缓存职责塞进 service worker。
  - `DashboardActivityOverview` 已接入 `live` / `cached-offline` / `not-cached-yet` 三态；`working conversations` 明确保留在线依赖，并在离线重开时显示不可用语义。
  - install icon 保留透明 regular 与独立 maskable 输出；`scripts/export_brand_assets.py` 为 favicon、regular 和 maskable 资源生成内容哈希文件名，`vite.config.ts` 只消费这组五个唯一资源。既有批准的 regular/maskable artwork 像素未改变。
  - `site.webmanifest`、`sw.js`、`version.json` 与 `index.html` 走重新校验，service worker 排除运行时元数据和安装图标后再 precache，内容哈希图标由后端静态服务使用 immutable 缓存；产品 App 不生成或注入 Apple touch fallback。

## 状态

- Status: 已实现
- Created: 2026-07-17
- Last: 2026-09-02

## 实现范围

### Runtime contract

- base-aware `site.webmanifest`
- service worker inject-manifest build
- browser-native install prompt + Safari manual guidance
- waiting-update prompt
- offline shell banner
- generated regular/maskable/favicon icon contract with content-hashed filenames
- revalidated PWA metadata and immutable install-icon caching

### Dashboard overview snapshots

- IndexedDB schema `cvm-dashboard-overview-snapshots`
- 五个固定 range 的最新成功快照写入 / 读取 / schema version 校验
- 在线首次渲染后顺序预热剩余 range
- 离线 / 网络类失败时的 cached fallback
- `not-cached-yet` 空状态与 `cachedAt` banner

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `python3 scripts/export_brand_assets.py && cd web && bun run test:pwa-assets`
- `cd web && bun run test`
- `cd web && bun run test-storybook`
- `cd web && bun run test:e2e:pwa`
- `cd web && bun run build`
- 构建产物级 DOM/HTML、manifest、响应缓存与 service worker precache/cache-first 排除回归
- Chromium V1 已安装到 V2 发布的 manifest/icon 更新回归，无需卸载或重新安装

## 文档更新（Docs Updated）

- `docs/specs/m9p2w-installable-pwa-runtime/SPEC.md`
- `docs/specs/m9p2w-installable-pwa-runtime/IMPLEMENTATION.md`
- `docs/specs/m9p2w-installable-pwa-runtime/HISTORY.md`
- `README.md`
- `web/README.md`
- `docs/deployment.md`
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
- `web/src/components/ui/dialog.tsx`
  - 为共享 dialog 补充 `mobileLayout="centered"`，让需要真实 modal 语义的 UI 不再被默认底部抽屉样式带偏。
- `web/src/features/app-shell/PwaInstallControl.tsx`
  - 改为纯 dialog surface：移除 trigger button，由 app shell 在 `prompt` / `manual-ios` 模式下自动拉起安装提示。
- `web/src/features/app-shell/AppLayout.tsx`
  - 头栏不再渲染 install/status button，改为按当前 PWA 安装状态自动展示一次性 prompt / guidance。
- `web/src/features/app-shell/PwaInstallControl.test.tsx`
  - 锁定自动安装提示的“无 trigger + 居中 modal + confirm action”契约，不允许回退成头栏按钮。
- `scripts/export_brand_assets.py` 与 `web/scripts/check-pwa-assets.py`
  - 以 traced product mark 导出独立 regular、maskable 与 favicon 资源，并检查尺寸、透明度、safe circle、内容哈希文件名、manifest purpose、DOM HTML 引用、响应缓存与 SW precache/cache-first 边界。
- `src/maintenance/hourly_rollups.rs`
  - 对 PWA 元数据和入口 HTML 返回重新校验策略，对内容哈希安装图标返回 immutable 缓存，并覆盖静态缓存分类单元测试。
- `web/tests/pwa/installable-runtime.spec.ts` 与 `web/tests/pwa/test-server.mjs`
  - 验证稳定 manifest identity、当前图标响应、缓存策略与 SW 不缓存运行时 manifest/version/安装图标，并模拟同一 Chromium registration 从 V1 更新到 V2 的新 manifest/icon URL。
- 本次变更不重绘 regular/maskable artwork；资源检查器还会将 favicon 与批准的 regular SVG 做字节级比对，发布验证需保留四个 regular/maskable PNG 的原始哈希。

## Visual Evidence

- Canonical owner-facing captures live in `docs/specs/m9p2w-installable-pwa-runtime/SPEC.md#visual-evidence`.
- Captured artifacts:
  - `./assets/pwa-install-prompt-mobile.png`
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
- [x] M6: 产品 App 移除 Apple touch fallback，并完成已安装 Chromium V1→V2 图标更新回归
