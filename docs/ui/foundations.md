# Foundations

## 当前真相源

### 主题机制

- 文档主题由 `web/src/theme/context.tsx` 管理，只支持 `light` 与 `dark` 两种 `ThemeMode`。
- DOM 上的主题真相是 `data-theme="vibe-light"` / `data-theme="vibe-dark"` 与 `data-color-mode="light|dark"`。
- 默认主题优先读取 `localStorage` 中的 `codex-vibe-monitor.theme-mode`，否则回退到系统 `prefers-color-scheme`。

### 语义色与表面层级

- `web/src/index.css` 中的 `:root` 与 `[data-theme='vibe-dark']` 定义了主题 token，项目统一使用语义 token，而不是直接在页面里硬编码品牌色。
- 基础语义色固定使用 `base / primary / secondary / accent / neutral / info / success / warning / error` 这一组命名。
- 页面背景不是纯色平铺，而是由渐变、orb 和细网格叠加构成；应用级 surface 再通过半透明面板与阴影抬起。
- 通用表面层级优先复用这些类：`surface-panel`、`drawer-shell`、`dialog-surface`、`sbdocs .sbdocs-preview`。

### 字体、数字与圆角

- 全站正文默认字体栈定义在 `body`：`IBM Plex Sans`、`Avenir Next`、`Segoe UI`、`PingFang SC`、`Hiragino Sans GB`、`Microsoft YaHei`、`sans-serif`。
- 默认启用 `font-variant-numeric: tabular-nums` 与 `font-feature-settings: 'tnum' 1`，数据密集场景要保持数字列宽稳定。
- 全局圆角语义来自 `--radius-selector`、`--radius-field`、`--radius-box`；实现层通常落在 `rounded-md`、`rounded-lg`、`rounded-xl`、`rounded-full` 这几个层级。

### 动效边界

- 全局元素默认只过渡 `background-color`、`border-color`、`color`、`box-shadow`，避免大范围 `transition: all`。
- Popover、Dialog、Storybook overlay 都使用短时长的 fade / zoom / slide 进入离场动画；页面文档不定义额外复杂 motion system。

## 后续新增规则

- 新增主题只能在现有 light/dark 双主题框架内演进；不要引入第三套主题或页面私有主题开关。
- 新增颜色 token 时必须先回答它属于语义层还是特定组件层；能落在语义层就不要创建 feature 私有色。
- 页面级容器优先复用已有 surface 语义；新增卡片、抽屉或浮层若只是视觉微差，优先通过现有类组合，不新增一组近似 token。
- spacing 虽然还没有独立 token 文件，但新增界面必须沿用现有 Tailwind 尺度：同一组件内的控件间距优先使用 `gap-2` / `gap-3`，区块级内容优先使用 `gap-4` / `gap-6`，大段页面分区优先使用 `space-y-6` / `space-y-8`。
- panel、card、dialog 的内边距默认从 `p-4` 起步；信息密度高的表格容器可以降到 `p-3`，大屏详情面板或设置分组可以提升到 `p-6`，不要在同一视图里混用过多离散 padding。
- 表单一组 label + control + hint/error 之间保持紧凑层级：字段内反馈优先 `space-y-1` 或 `space-y-2`，字段组之间优先 `space-y-4`，避免靠空白把表单拉成过长页面。
- 响应式收缩时先减少外围留白，再减少区块间距；移动端默认保留 `px-4` 级别的页面边距，不要为了塞内容直接压到贴边。
- 数字、金额、token、延迟等密集数据默认使用等宽数字语义，确保表格与 KPI 在 light/dark 下都稳定对齐。
- 任何全局动画新增都必须证明比当前 `180ms ~ 220ms` 的轻量反馈更必要，否则默认沿用现有节奏。

## 已知例外 / 待治理

- 当前 spacing 没有单独导出为命名 token，更多依赖 Tailwind 的 `gap-*`、`p-*`、`px-*`；文档只能约束常用层级，不能像完整 design token 系统一样提供一套命名尺度。
- 某些表面层级同时依赖 CSS 变量和 utility class，意味着主题迁移仍有耦合成本。
- 全局没有单独的 motion/accessibility token；如果后续引入 reduced motion 适配，需要补一份跨组件策略。
