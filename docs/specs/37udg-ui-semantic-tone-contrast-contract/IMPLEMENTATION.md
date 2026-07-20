# UI semantic tone contrast contract - Implementation

## Current State

- Canonical spec: `docs/specs/37udg-ui-semantic-tone-contrast-contract/SPEC.md`
- Implementation summary: 已完成 semantic tone-ink contract 落地、共享/业务组件迁移、Storybook dark 覆盖、source contract test 与视觉证据收口。
- Delivery scope: semantic tone-ink token、shared badge/marker/metric/chip migration、Storybook dark coverage、contract test、设计文档同步。

## Implemented Notes

- `web/src/index.css`
  - 新增 `tone-ink-{primary|accent|info|success|warning|error}` shared token / utility class。
  - 在 light/dark 双主题里明确 filled-content 与 tone-ink 的边界。
- `web/src/components/ui/badge.tsx`
  - shared low-opacity semantic variants 统一改走 tone-ink contract。
- `web/src/features/invocations/InvocationWorkflowDetailPanel.tsx`
  - 修复 snapshot metric、summary row、timeline marker、divider 与 action link 的 dark contrast。
- `web/src/features/app-shell/AppLayout.tsx`
  - 修复 SSE offline banner 与 PWA offline banner chip 的 warning tone text。
- `web/src/features/app-shell/PwaInstallControl.tsx`
  - 修复 offline chip 的 warning tone text。
- Storybook
  - 更新 invocation detail / app shell / PWA stories，并新增 badge semantic gallery。
  - 新增 `FailedPoolWorkflowDark`、`BlockedPoolWorkflowDark`、`PwaOfflineBannerDark`、`SemanticToneGalleryDark` dark regression surface。
- Tests
  - 新增 `web/src/components/ui/semantic-tone.contract.test.ts`，拦截低透明语义底继续误配 `text-*-content`。
- Docs
  - 更新 `DESIGN.md`、`docs/ui/foundations.md`、`docs/ui/components.md` 与 `docs/specs/README.md`。

## Quality Gates

### Testing

- `cd web && bun run test -- src/components/ui/semantic-tone.contract.test.ts src/features/app-shell/AppLayout.test.tsx src/features/invocations/InvocationWorkflowDetailPanel.test.tsx`
- `cd web && bun run test-storybook`
- `cd web && bun run build`
- `rg -n -P "bg-(primary|accent|info|success|warning|error)/(?:[1-9]|[1-6][0-9]|7[0-9])(?!\\d).*text-(primary|accent|info|success|warning|error)-content|text-(primary|accent|info|success|warning|error)-content.*bg-(primary|accent|info|success|warning|error)/(?:[1-9]|[1-6][0-9]|7[0-9])(?!\\d)" web/src`

### Visual verification

- Storybook dark failed invocation detail capture
- Storybook dark blocked invocation detail capture
- Storybook dark semantic badge gallery capture
- Storybook dark PWA / app-shell offline state capture

## Docs Updated

- `DESIGN.md`
- `docs/ui/foundations.md`
- `docs/ui/components.md`
- `docs/specs/README.md`
- `docs/specs/37udg-ui-semantic-tone-contrast-contract/SPEC.md`
- `docs/specs/37udg-ui-semantic-tone-contrast-contract/IMPLEMENTATION.md`
- `docs/specs/37udg-ui-semantic-tone-contrast-contract/HISTORY.md`

## Delivery Checklist

- [x] M1: 建立 semantic tone-ink token 与 utility contract。
- [x] M2: 迁移调用详情、共享 badge、offline chip / banner。
- [x] M3: 补齐 Storybook dark scenarios 与 contract test。
- [x] M4: 同步设计文档与 spec 索引。
- [x] M5: 完成本地验证与视觉证据。
