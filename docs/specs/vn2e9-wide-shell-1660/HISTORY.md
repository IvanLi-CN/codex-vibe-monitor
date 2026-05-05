# 全站 1660 宽屏壳层适配 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/vn2e9-wide-shell-1660/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-04-06: 新建 spec，冻结 1660 宽屏 shell 契约、路由覆盖范围、Storybook / Playwright 验收门槛与视觉证据归档路径。
- 2026-04-06: 本地实现完成：共享壳层宽度契约落地到 AppLayout / Update banner / 顶层路由；Storybook 页面级宽屏入口、`wide-shell-layout` E2E 与最终视觉证据已补齐，本地 `build + test + build-storybook + targeted e2e` 通过，等待截图提交授权后进入 PR 收敛。
- 2026-04-07: 主人已确认视觉结果可继续，`gpt-5.4` review-loop 未发现可执行阻塞项；本地 `build + test + build-storybook + targeted e2e` 维持通过，PR #298 已打 `type:minor` + `channel:stable` 并收敛到 merge-ready。
- 2026-04-07: 为清除 PR freshness gate，同步 `origin/main` 并重新生成 Storybook 宽屏截图；最新工作树在同步后再次通过 `build + test + build-storybook + targeted e2e`，视觉证据已刷新到当前待推送 head。
- 2026-04-07: fresh review 指出页面级 Storybook surface 外层 gutter 会让 `desktop1660` 少于真实壳层宽度；已把横向 padding 收回 `app-shell-boundary` 内部，并再次重跑 `build + test + build-storybook + targeted e2e`，同时刷新宽屏截图到最新工作树。
- 2026-04-07: 本地按 CI workflow 复现 `Lint & Format Check`，确认失败点是 `web/src/components/storybookPageHelpers.tsx` 触发 `react-refresh/only-export-components`；已把 `jsonResponse` 抽到独立 `storybookResponse.ts`，并重新通过 `cargo check --locked --all-targets --all-features`、`bun run check:bun-first`、`cd web && bun run lint`、`cd web && bun run storybook:build -- --quiet`、`cd docs-site && DOCS_BASE=/codex-vibe-monitor/ bun run build` 与 `assemble-pages-site.sh`。
- 2026-04-07: fresh merge-proof review 发现更新横幅在 `1680px` 临界宽度下会比共享壳层窄 `32px`；已把 `app-shell-banner-boundary` 改回与主壳层同宽，并把 `wide-shell-layout` 回归扩展到 `1680` 视口，同时补充深色主题下的本地预览视觉证据。
- 2026-04-07: 跟进 fresh review 继续补齐子 `1660px` 视口场景：更新横幅改为“小屏保留 `16px` 外边距、`1660px` 及以上与壳层同宽”的分段契约，并把 `wide-shell-layout` 回归扩展到 `1024` 视口，同时补充深色主题下的 `1024px` 本地预览视觉证据。
