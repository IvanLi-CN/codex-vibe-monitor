# UI surface 对比层级收口 - Implementation

## Current State

- Canonical spec: `docs/specs/x4v2n-ui-surface-contrast-hierarchy/SPEC.md`
- Implementation summary: 已完成。
- Delivery scope: Web UI surface token、shared primitives、Dashboard / Settings / Account Pool 高可见度 surface、设计文档同步。

## Implementation Notes

- `web/src/index.css` 新增共享 surface token 与 class：
  - `surface-card`
  - `surface-subtle`
  - `surface-inset`
  - `field-surface`
  - `menu-surface`
  - `dialog-chrome-surface`
  - `destructive-callout-*`
- `Card` 默认迁移到 `surface-card`，`Input` / `SelectTrigger` 迁移到 `field-surface`，`SelectContent` 迁移到 `menu-surface`。
- Dashboard 批量路由绑定与清理确认 dialog 的 sticky header/footer 使用 `dialog-chrome-surface`；清理确认 callout 使用低饱和 destructive token。
- Settings 高密度配置卡、forward proxy 区块、桌面表格和移动卡片复用 `surface-inset` / `surface-subtle`，减少页面级高透明底色和亮边框。
- Account Pool 高可见度能力卡、路由设置弹窗、详情抽屉 card 和 dialog chrome 改用共享 surface class。
- `DESIGN.md` 与 `docs/ui/*` 更新共享 surface vocabulary 和新增规则。

## Quality Gates

### Testing

- `cd web && bunx vitest run --project=unit src/features/dashboard/DashboardWorkingConversationsSection.test.tsx`
- `cd web && bunx vitest run --project=unit src/features/dashboard/DashboardWorkingConversationsSection.test.tsx src/pages/Settings.test.tsx src/pages/account-pool/UpstreamAccounts.test.tsx`
- `cd web && bun run build`

### Visual verification

- Web Demo dark Dashboard capture: bulk clear/reset destructive dialog and surrounding dashboard surface hierarchy.
- Web Demo dark Settings capture: dense Settings cards and field surfaces.
- Web Demo light Account Pool capture: card/detail surface hierarchy.
- Storybook note: Storybook preview was not used because the current preview build is blocked by an unrelated React Refresh duplicate-symbol error in `src/theme/context.tsx`.

## Docs Updated

- `DESIGN.md`
- `docs/ui/README.md`
- `docs/ui/foundations.md`
- `docs/ui/components.md`
- `docs/ui/patterns.md`
- `docs/specs/README.md`
- `docs/specs/x4v2n-ui-surface-contrast-hierarchy/SPEC.md`
- `docs/specs/x4v2n-ui-surface-contrast-hierarchy/IMPLEMENTATION.md`
- `docs/specs/x4v2n-ui-surface-contrast-hierarchy/HISTORY.md`

## Delivery Checklist

- [x] M1: 定义共享 surface token 与 class。
- [x] M2: 迁移 Card/Input/Select primitive 默认 surface。
- [x] M3: 收口 Dashboard、Settings、Account Pool 高可见度 surface。
- [x] M4: 同步 DESIGN 与 docs/ui 规则。
- [x] M5: 完成本地单测、build 与 Web Demo 视觉验证。
