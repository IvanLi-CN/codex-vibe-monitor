# Storybook

## 当前真相源

### 全局运行约束

`web/.storybook/preview.ts` 定义了当前 Storybook 的运行基线：

- 全局引入 `web/src/index.css`，所以 Storybook 画布与应用主题保持同一套样式源。
- 使用 `ThemeProvider` 与 `StorybookThemeBridge` 驱动 light/dark 预览。
- 默认布局是 `fullscreen`。
- 默认 viewport 是 `desktop1280`，可切换到 `mobile390`、`mobile430`、`tablet768`、`laptop1024`、`desktop1440`。
- docs 画布表面已被改造成跟应用同一套 surface 语言，而不是 Storybook 默认白底文档皮肤。

### 推荐作为真相源的 stories

- Shell / Layout：`web/src/components/AppLayout.stories.tsx`
- Settings：`web/src/components/SettingsPage.stories.tsx`
- Records：`web/src/components/RecordsPage.stories.tsx`
- Invocation list：`web/src/components/InvocationTable.stories.tsx`
- Dashboard KPI：`web/src/components/TodayStatsOverview.stories.tsx`
- Tags / account-pool 页面：`web/src/components/TagsPage.stories.tsx`
- 基础输入组件：`web/src/components/ui/filterable-combobox.stories.tsx`、`web/src/components/ui/form-field-feedback.stories.tsx`、`web/src/components/ui/info-tooltip.stories.tsx`

这些 story 不只是演示，它们也是当前页面结构、状态语义与视觉证据的重要事实来源。

### 证据采集口径

- 页面级视觉确认优先从 Storybook 或浏览器 smoke 里拿证据，而不是从实现截图中二次猜测布局。
- 采集证据时至少覆盖一个桌面 viewport；涉及移动端差异时，再补 `mobile390` 或 `mobile430`。
- 有主题差异的组件，默认要在 light/dark 两种主题下都能复核。

## 后续新增规则

- 新增通用组件或页面模式时，优先补 story，再在 `docs/ui/` 回链该 story 作为可复核入口。
- 任何 story 如果承担“视觉真相源”角色，就要保证数据、文案和状态足够稳定，不依赖真实网络。
- 页面 story 应优先 mock API、SSE、session storage 与 router，而不是要求人工准备后端环境。
- 新视觉证据要尽量沿用现有 viewport 命名，避免每个 feature 发明一套自己的截图尺寸口径。

## 已知例外 / 待治理

- 不是所有 `web/src/components/ui/` 组件都有独立 story；目前仍有部分组件依赖页面 story 间接验证。
- 个别交互细节仍需浏览器真实环境复核，Storybook 只能提供大部分视觉与结构证据。
- 现有 stories 的命名与层级已足够支撑文档，但还没有单独的“UI guideline showcase”合集页。
