# UI 规范总览

`docs/ui/` 是本仓库的内部全局 UI 文档入口，用来沉淀已经落地的视觉与交互真相源，并为后续新增页面、组件和 Storybook 证据提供统一约束。面向协作者的 public docs 入口位于 `docs-site/docs/`；该目录继续保留更细的内部规范与事实来源。

## 文档地图

- [`foundations.md`](./foundations.md)：主题、设计 token、字体、圆角、表面层级与动效边界。
- [`components.md`](./components.md)：基础组件、表单反馈、状态语义与最小可访问性要求。
- [`patterns.md`](./patterns.md)：页面级布局模式、列表/详情、空态/加载/错误态与响应式规则。
- [`data-viz.md`](./data-viz.md)：图表、热力图、指标色、数字对齐与数据展示约定。
- [`storybook.md`](./storybook.md)：Storybook 主题切换、viewport、证据采集口径与推荐 story。

## 当前真相源

- 主题与全局样式：`web/src/index.css`
- 主题切换机制：`web/src/theme/context.tsx`
- 图表配色：`web/src/lib/chartTheme.ts`
- Storybook 运行约束：`web/.storybook/preview.ts`
- public docs 文档壳：`docs-site/docs/`
- 基础组件实现：`web/src/components/ui/`
- 页面级参考：`web/src/components/AppLayout.stories.tsx`、`web/src/components/SettingsPage.stories.tsx`、`web/src/components/RecordsPage.stories.tsx`、`web/src/components/InvocationTable.stories.tsx`、`web/src/components/TodayStatsOverview.stories.tsx`
- 历史功能 spec：`docs/specs/jpg66-settings-shadcn-refresh/SPEC.md`、`docs/specs/6whgx-records-stable-snapshot-analytics/SPEC.md`、`docs/specs/g4ek6-account-pool-upstream-accounts/SPEC.md`

当文档和实现不一致时，先以实现与对应 story 为准，再回写本目录与相关 spec；不要让 `docs/ui/` 先于真实实现漂移。

## 后续新增规则

- 新增基础样式 token、组件状态或通用布局模式时，必须先判断应该补到哪一份文档，而不是只写在单个 feature spec 里。
- 新增可复用组件时，优先补充对应 Storybook story，再在 `components.md` 或 `patterns.md` 里补使用约束。
- 若 public docs 中的 Storybook 导览或入口口径发生变化，需同步更新 `docs-site/docs/storybook.mdx` 与 `docs-site/docs/storybook-guide.mdx`。
- 新增图表或新指标颜色时，先复用 `data-viz.md` 既有语义；只有现有语义不足以表达时，才扩展 token 与规范。
- 新增页面若形成新的通用交互模式，必须把通用规则抽到 `patterns.md`，避免规范继续散落在页面实现里。

## 已知例外 / 待治理

- 当前规范仍有一部分事实来自 feature story 与页面故事，而不是独立 design token 层；这代表文档已经覆盖现状，但设计系统尚未完全抽象化。
- 现有颜色、间距与表面层级同时存在 CSS 自定义属性和 Tailwind utility 双来源，后续若继续扩展主题，需评估是否补一层集中 token 映射。
- 某些 feature spec 已经记录页面级视觉证据，但尚未统一回链到本目录；后续新增 spec 时应优先链接回 `docs/ui/` 的对应章节。
