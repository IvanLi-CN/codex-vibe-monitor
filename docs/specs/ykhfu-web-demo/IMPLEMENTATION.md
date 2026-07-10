# 全产品路由 Web Demo 实现状态（#ykhfu）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实。

## Current Status

- Implementation: 已完成并发布
- Lifecycle: active
- Catalog note: Browser-only demo runtime and Pages delivery shipped in `v2.22.0`; live runtime remains backend-bound and does not initialize MSW.

## Coverage / rollout summary

- `web/src/main.tsx` 在 demo render 前启动 worker；worker 初始化或未知 runtime 失败时显示受控错误面，并对非 asset 未处理请求 fail closed。
- `web/src/demo/` 提供 deterministic seed、HTTP/SSE handlers、内存 mutation、四个 scene，以及桌面 Inspector 与移动 drawer。
- `web/src/demo/DemoInspector.stories.tsx` 提供 autodocs state entry 与 scene switch play coverage。
- `AppLayout` 补充移动端的紧凑导航布局与 Storybook mobile state，确保 demo 的 390px 页面证据保持可导航。
- Pages assembly 把 `web/demo-dist/` 放入 `/demo/`。demo API/SSE、MSW worker 和 public branding assets 全部使用 `VITE_DEPLOY_BASE` 的 repo-subpath，避免 Pages worker scope 外溢到站点根 API。
- `Records Overlay E2E` 在保留原 live Vite regression 的同时运行 demo route matrix、Inspector sharing/SSE 与 simulated external-key write。
- `SPEC.md` 的 `## Visual Evidence` 保存 Dashboard operational、账号池 attention 与 Records network-failure 的桌面和移动 mock-only 证据，绑定 `8b0aa929b9738fd0d535784e4b89c75ce54e28ae`。
- PR #582 已合并为 `main@80a248cc891f74111f9d403c9d915a1f340a72d5`，并发布为 `v2.22.0`。

## Remaining Gaps

- None

## Related Changes

- PR #582: `feat(web): add mock-only product demo`
- Release: `v2.22.0`
- Verification: `bun run lint`, `bun run test`, `bun run test-storybook`, `bun run demo:build`, `bun run storybook:build`, Pages assembly smoke, route matrix E2E and Records Overlay E2E.

## References

- `./SPEC.md`
- `./HISTORY.md`
