# 移动优先导航与浮层收口实现状态（#hnu7b）

> 当前有效规范以 `./SPEC.md` 为准；此文记录已落地的实现边界和验证事实。

## Current Status

- Implementation: completed locally
- Lifecycle: active
- Branch: `th/mobile-adapt-navigation`
- Base: `origin/main@2381798fffb05288bacfb007ab9ef0901a040019`

## Implemented Coverage

- `useCompactViewport`、`AppLayout` 与导航树以 `768px` 作为紧凑/桌面分界；紧凑导航抽屉包含一级与二级路由。
- 通用 `Dialog` 和账号详情 drawer shell 在紧凑视口使用安全区友好的 bottom sheet，在桌面恢复既有 dialog/side drawer。
- 上游账号详情和 Prompt Cache 会话详情支持紧凑页面化；账号详情 URL 继续使用既有账号与 tab 参数。
- Settings、External API keys、账号池操作、调用详情和其他相关表面已接入响应式浮层规则。
- 关键表格在窄屏改为可扫描的 card/list，筛选与子导航收敛为窄屏结构。
- Records 的筛选编辑改为共享响应式 drawer：桌面为右侧工作抽屉，紧凑视口为全高 bottom sheet；页面主体只显示已提交的筛选条件，并支持逐项移除后立即刷新快照。
- Dashboard 今天活动图在紧凑视口聚合密集分钟柱、保留失败向下的符号语义、移除移动端延迟轴；当前对话区将说明与 workspace controls 分行，避免窄列断行。
- Dashboard KPI 网格在 `400px` 起切换为两列，Token 等长数值跨满两列；窄于该阈值保持单列，桌面既有多列网格不变。
- Dashboard 活动总览在紧凑视口将时间范围与指标 segmented controls 收口为左右并排的两个下拉；复用既有状态与数据加载 contract，宽屏保留 segmented controls。
- 所有页面级 `surface-panel` 在移动端扁平化为结构容器，保持单一 `12px` 页面 gutter；Settings 与 External API Keys 的外层 Card 同步扁平化，内部数据项保留紧凑 card。
- 移动端移除桌面装饰背景；Dashboard Working Conversations 不再向扁平页面容器重复注入内边距。
- App shell、账号详情、Dashboard、Settings 和 Prompt Cache 的 Storybook 移动状态已更新。

## Verification

- `bun run lint` passed.
- `bun run build` and `bun run demo:build` passed.
- Records targeted Vitest coverage passed: 40 tests across page and hook behavior.
- Records Storybook entry includes the `mobile390` filter drawer interaction state.
- Dashboard chart and current-conversation mobile regression coverage passed: 93 tests.
- Unit Vitest report: 276 suites and 1191 tests passed.
- `bun run test-storybook` passed: 4 files and 6 tests, with 52 unsupported browser-only stories skipped by the configured project.
- `bun run build-storybook` passed.
- Web Demo evidence covers `320 / 390 / 430 / 768` vertical viewports and is bound to `15089c374ffab1dd0669aae78de3279148140cdb`.

## Remaining Delivery Steps

- Complete fast-flow review, PR creation, CI convergence, and merge-ready closure.

## References

- `./SPEC.md`
- `./HISTORY.md`
