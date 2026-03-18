# Patterns

## 当前真相源

### App shell 与导航

- 站点骨架由 `AppLayout` 代表，参考 `web/src/components/AppLayout.stories.tsx`。
- 顶部导航和分段控件使用统一的 active/inactive 语言：圆角胶囊、轻边框、选中态背景抬升、文字权重提升。
- 页面内容默认放在带 breathing room 的容器中，避免贴边或全宽压迫布局。

### 面板、卡片与详情区

- 仪表盘、设置、记录页与标签页都偏向 surface-first 布局：先给一层卡片/面板，再在内部使用 section heading、divider 与 metric grid。
- 抽屉优先使用 `web/src/index.css` 里的 `drawer-shell`、`drawer-header`、`drawer-body`；模态对话框优先使用 `web/src/components/ui/dialog.tsx` 提供的 `Dialog` 体系，不重复实现浮层外壳。
- KPI 与 summary 卡片应优先采用“标签小、数字大、辅助说明轻”的三段结构。

### 列表 / 表格 / 详情展开

- 数据密集列表同时兼顾桌面表格与移动端卡片/堆叠布局，代表参考是 `web/src/components/InvocationTable.stories.tsx`、`web/src/components/InvocationRecordsTable.stories.tsx`、`web/src/components/UpstreamAccountsPage.stories.tsx`。
- 详情信息优先以内联展开、抽屉或卡片二级区块呈现，不鼓励跳转到无上下文的新页面。
- 长文本、代理名、endpoint、token key 等字段默认允许截断，但必须保留可在详情区或 tooltip 中复核的路径。

### 表单区块

- 表单默认按逻辑分组，而不是单字段漫灌；设置页 story 是当前高密度表单的主参考。
- 二列布局只在 `md+` 视口展开，移动端默认回到单列，避免并排字段压缩可读性。
- 布尔开关旁边应同时出现标题与简要说明，不能只剩一个裸 switch。

### 空态 / 加载 / 错误态

- 空态：保留原布局骨架，用说明文案明确“暂无数据/未配置/还未搜索”。
- 加载态：优先 skeleton、spinner 或稳定占位，不要因为请求刷新让整个卡片高度跳变。
- 错误态：保留上下文标题，再用 `Alert` 或错误块说明问题，避免把页面完全替换成一行报错。

### 响应式规则

- Storybook 默认桌面 viewport 是 `desktop1280`，移动端常用 `mobile390` / `mobile430`；文档、页面与视觉验收都以这组断点为主。
- 页面级容器一般在桌面限制最大宽度，移动端改为单列拉伸；不要把桌面三列布局硬压到手机上。
- 任一交互浮层在移动端如果无法稳定覆盖，应优先退化为更稳的堆叠/抽屉/原地展开模式，而不是继续叠 z-index。

## 后续新增规则

- 新页面应先判定自己属于“仪表盘总览”“数据列表分析”“设置表单”“模块管理台”中的哪一类，再复用对应现有模式。
- 新的页面级 pattern 只有在被两个以上场景复用时才进入本文件；单次 feature 特例仍保留在 feature spec 中。
- 响应式设计先保证信息架构不丢失，再考虑桌面视觉丰满度；移动端退化应是设计好的收敛，而不是简单隐藏信息。
- 同一个页面内若同时存在实时数据、筛选与详情，优先保持“筛选在前、列表在中、详情在近处”的结构，减少跨区域跳转。

## 已知例外 / 待治理

- 个别历史页面的 spacing 与 section heading 还没有完全统一到当前 shell 语言，属于遗留差异，不应被当成新页面模板继续复制。
- 某些记录页交互依赖 feature-specific 状态机与懒加载逻辑，文档只能定义稳定模式，不能代替具体行为 spec。
- 还没有单独抽象出“页面模板组件”层，当前 pattern 更多依赖 story 与类名约束。
