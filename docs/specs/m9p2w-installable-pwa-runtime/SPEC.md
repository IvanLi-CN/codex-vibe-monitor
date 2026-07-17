# Installable PWA 运行时与应用壳层交付（#m9p2w）

> 当前有效规范以本文为准；实现覆盖与当前状态见 `./IMPLEMENTATION.md`，关键演进原因见 `./HISTORY.md`。

## 背景 / 问题陈述

- 现有前端已经维护 `apple-touch-icon`、app icons 与 standalone identity，但仍停留在“metadata-only”层：没有正式 service worker、没有 prompt-style update UX，也没有对 Safari / iOS 的手动安装路径给出 owner-facing 指引。
- `HashRouter` 应用在桌面浏览器里可正常工作，但当前 manifest `start_url`、install entry、update 行为与离线壳层都没有被当成正式 runtime contract 维护。
- `style-playbook` 已经把 `Codex Vibe Monitor` 标记成 `Progressive web app` topic 样本；如果实现层继续停在 metadata-only，topic 与项目 current truth 会持续错位。

## 目标 / 非目标

### Goals

- 把 `Codex Vibe Monitor` 提升为 `installable-runtime` 成熟度：manifest、service worker、install affordance、prompt-style update 与离线壳层成为正式前端交付面。
- 保持现有 `HashRouter`、REST/SSE、工作台信息架构和桌面视觉语言不回退，让安装后的默认入口稳定落到 `/#/dashboard`。
- 把桌面 Chromium、Android Chrome 与 Safari / iOS manual guidance 收口为明确 browser matrix，并通过专用 PWA 测试面验证。
- 把 owner-facing 文档、spec 和 style-playbook snapshot 同步到“installable-runtime，但不是 offline-capable 数据应用”的统一口径。

### Non-goals

- 不把产品提升为 `offline-capable` 数据应用；离线时不承诺真实 API 数据、SSE 或设置写回继续可用。
- 不重构主应用信息架构、导航树、后端接口、鉴权模型或代理业务语义。
- 不引入原生封装、推送通知、后台同步或独立的移动端手势体系。

## 范围（Scope）

### In scope

- `vite-plugin-pwa` `injectManifest` 路线、base-aware manifest、service worker 与 `version.json` 更新检测。
- App shell install control、Safari manual guidance、installed-state vocabulary、prompt-style update banner 与 browser offline banner。
- PWA 专用 Vitest / Storybook / Playwright 覆盖，以及项目文档和 playbook current truth 更新。

### Out of scope

- API 数据缓存正确性、离线写操作恢复、后台任务离线排队。
- 与 installable PWA 无关的图表、表格或页面重设计。

## 功能与行为规格

### Install surface

- 应用必须生成 base-aware manifest，包含稳定 identity、icons、theme color、`start_url=./#/dashboard`、`scope=./` 和高价值 shortcuts。
- 安装入口必须在共享 app shell 中可达；Chromium / Android Chrome 使用 `beforeinstallprompt`，Safari / iOS 提供 manual Add to Home Screen guidance。
- 已安装状态必须使用独立 vocabulary，不再继续显示“可安装”入口。

### Update behavior

- 前端 service worker 必须采用 prompt-style update；检测到 waiting worker 后，由用户明确触发刷新，禁止 mid-session 自动 takeover。
- 更新 banner 必须与现有版本 vocabulary 保持一致，至少显示当前前端版本与即将切换的新版本。
- `version.json` 不能被旧 worker 静态 precache 吞掉；更新提示读取的新版本号必须来自网络真相。

### Offline shell

- 首次在线访问成功后，关闭网络仍可打开应用壳层与基础静态资源。
- 离线时必须显示 owner-facing banner，明确“壳层仍可打开，但实时数据、SSE 与设置同步暂停”。
- `/api/*`、`/events` 与测试控制路径不得被 service worker 导航 fallback 误拦截。

## 验收标准

- Given 桌面 Chromium 或 Android Chrome
  When 页面满足安装条件
  Then app shell 显示明确 install affordance，并在用户确认后切换到 installed-state vocabulary。

- Given 新版本前端资源已经部署
  When 当前页面检测到 waiting service worker
  Then 页面显示 prompt-style update banner，且只有用户点击更新后才 reload 到新壳层。

- Given 浏览器已成功在线访问并完成 shell 缓存
  When 网络断开并重新打开 `/#/dashboard`
  Then 应用壳层可继续加载，并显示明确的 offline/data-unavailable 说明。

- Given Safari / iOS
  When 用户尝试安装
  Then UI 不伪装成存在原生 install prompt，而是提供手动 Add to Home Screen 指引。

## 非功能性验收 / 质量门槛

### Testing

- `cd web && bun run build`
- `cd web && bun run test`
- `cd web && bun run test-storybook`
- `cd web && bun run test:e2e:pwa`

### UI / Storybook

- `Shell/PWA Install Control` 必须覆盖 prompt、Safari manual guidance 与 installed/offline 三种稳定状态。
- 视觉证据至少覆盖 header install affordance、update banner 与 offline banner。

## Visual Evidence

- Evidence source: `storybook-static` + local PWA preview/test server; no login, production account, secret, or live backend payload was used.
- Bound source revision: working tree on branch `th/pwa-installable-runtime` after the validated PWA runtime changes captured on 2026-07-17.
- Viewport: desktop `1440x1000`.
- Captures cover the shared install affordance, Safari manual guidance, prompt-style update banner, and offline shell degradation.

### Desktop

![App-shell install affordance](./assets/pwa-install-prompt-desktop.png)

![Safari manual Add to Home Screen guidance](./assets/pwa-safari-manual-desktop.png)

![Prompt-style update banner](./assets/pwa-update-banner-desktop.png)

![Offline shell degradation banner](./assets/pwa-offline-banner-desktop.png)

## 参考（References）

- `web/vite.config.ts`
- `web/src/pwa/sw.ts`
- `web/src/hooks/usePwaRuntime.ts`
- `web/src/features/app-shell/PwaInstallControl.tsx`
- `web/playwright.pwa.config.ts`
- `web/tests/pwa/installable-runtime.spec.ts`
- `docs/specs/hnu7b-mobile-first-navigation-and-overlays/SPEC.md`
