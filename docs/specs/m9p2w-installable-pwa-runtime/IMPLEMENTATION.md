# Installable PWA 运行时与应用壳层交付 - Implementation

## Current State

- Canonical spec: `docs/specs/m9p2w-installable-pwa-runtime/SPEC.md`
- Implementation summary: 前端通过 `vite-plugin-pwa` `injectManifest` 生成 `site.webmanifest` 与自定义 service worker；app shell 新增 install control、Safari manual guidance、installed-state vocabulary、prompt-style update banner 与 browser offline banner；PWA 专项验证面包含 Vitest hook tests、Storybook stable states 与独立 Playwright suite。

## 状态

- Status: 已实现
- Created: 2026-07-17
- Last: 2026-07-17

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cd web && bun run build`
- `cd web && bun run test`
- `cd web && bun run test-storybook`
- `cd web && bun run test:e2e:pwa`

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新 spec 入索引
- `README.md`: installable PWA 浏览器合同与安装说明
- `web/README.md`: PWA 构建、测试与 Safari guidance
- `docs/specs/hnu7b-mobile-first-navigation-and-overlays/SPEC.md`: 原“PWA/offline 非目标”边界改为由本 spec 接管

## 实现前置条件（Definition of Ready / Preconditions）

- browser matrix 已锁定为 `Chromium Desktop + Android Chrome + Safari guidance`
- `offline-capable` 数据承诺明确不在本次范围内
- 更新策略已锁定为 prompt-style / next-launch，而不是自动接管

## Visual Evidence

- Canonical owner-facing captures live in `docs/specs/m9p2w-installable-pwa-runtime/SPEC.md#visual-evidence`.
- Captured artifacts: `./assets/pwa-install-prompt-desktop.png`, `./assets/pwa-safari-manual-desktop.png`, `./assets/pwa-update-banner-desktop.png`, `./assets/pwa-offline-banner-desktop.png`.

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: PWA build/runtime contract 落地
- [x] M2: app shell install/update/offline UX 落地
- [x] M3: PWA 专项验证面通过
- [x] M4: specs / project docs / playbook current truth 同步完成
