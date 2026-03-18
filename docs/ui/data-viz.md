# Data Visualization

## 当前真相源

### 图表色语义

`web/src/lib/chartTheme.ts` 是图表配色的唯一真相源，当前固定三类指标色：

- `totalCount`：蓝色系，表示请求量/次数。
- `totalCost`：橙色系，表示成本与价格。
- `totalTokens`：青绿色系，表示 token 体量。

这三类颜色同时覆盖折线、柱图、热力图与交互高亮，不为单个图表重新定义一套主题。

### 状态色与饼图

- 成功/失败状态色通过 `chartStatusTokens()` 提供；不要在图表里绕开这组 token 自己指定 success/failure 颜色。
- 饼图使用 `piePalette()` 提供的有序调色板，保证 light/dark 下的层次一致。

### 热力图梯度

- `heatmapLevels()` / `calendarPalette()` 为 count/cost/token 各自提供五档梯度。
- 热力图浅色模式从低对比灰到指标强调色，深色模式从低亮底色到高饱和强调色，目的是在暗背景下仍能看出层次。

### 数据展示与数字对齐

- KPI、表格数字、tooltip 数值默认使用等宽数字语义。
- token、cost、latency 这类指标在表格里应右对齐或使用统一的数字列视觉节奏，避免列宽跳动。
- `TodayStatsOverview`、`InvocationTable`、`RecordsPage` stories 是当前数据展示模式的主要参考。

### 交互与 tooltip

- 图表 hover / focus / tap 的浮层优先复用项目内 tooltip 语义，不引入浏览器原生 title 作为正式交互。
- 接近边缘时，tooltip 要以内收为先，不能因为视觉样式遮挡数据点或被容器裁切。

## 后续新增规则

- 新图表必须先判断它属于 count、cost、token 里的哪种主指标；主色只能复用现有语义，不新增第四套主色轴。
- 同一图表同时展示多类指标时，颜色优先级为：count 蓝、cost 橙、token 青绿；不要在不同页面交换语义。
- 新增状态图或 error breakdown 时，先复用现有 success/failure 与 pie palette；只有明确需要表达新状态层级时，才扩展 token。
- 表格中所有数字型字段都应优先保持对齐；若移动端改为卡片，也要保持 label/value 的稳定左右关系。
- 图表的 loading、empty、error 必须保留容器高度或布局骨架，避免因为数据刷新造成大面积跳动。

## 已知例外 / 待治理

- 当前图表 token 主要集中在 `web/src/lib/chartTheme.ts`，与全局 CSS 语义 token 之间还没有单独的中间映射层。
- 某些页面故事同时承担交互验证与视觉证明职责，说明图表规范还不够完全组件化。
- 如果未来新增更多财务或配额维度，需要先评估是扩充现有 cost/token 语义，还是建立新的二级图例规则。
