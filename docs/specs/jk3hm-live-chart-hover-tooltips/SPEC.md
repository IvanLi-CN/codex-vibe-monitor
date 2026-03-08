# Live 小图表悬浮详情统一升级（#jk3hm）

## 状态

- Status: 部分完成（4/5）
- Created: 2026-03-08
- Last: 2026-03-08

## 背景 / 问题陈述

- `Live` 页面已有 3 类高密度微图表：代理 24h 请求量柱图、代理 24h 权重趋势图、Prompt Cache 24h Token 累计 sparkline。
- 当前部分微图仍依赖浏览器原生 `title` / SVG `<title>`，在暗色高密度表格中视觉割裂，且定位不可控、无法统一高亮当前数据点，也无法提供稳定的触屏与键盘交互。
- 如果不统一 tooltip 交互，用户在 Live 页排查节点与对话细节时会持续受到原生 tooltip 延迟、遮挡和样式不一致影响。

## 目标 / 非目标

### Goals

- 为 Live 页上述 3 类微图引入统一的自定义 tooltip 原语与交互模型。
- 保持现有后端 API、聚合口径与图表数据结构不变，仅升级前端交互和视觉呈现。
- 支持桌面悬停、触屏点按、键盘聚焦后用方向键切换明细，并为当前数据点提供高亮与导引线。
- Tooltip 风格与现有暗色玻璃卡片语言一致，避免浏览器原生 tooltip 与自定义 tooltip 双开。

### Non-goals

- 不改造 `Live` 页之外的 Recharts 大图与其它页面图表。
- 不新增后端字段、SSE 事件或数据库变更。
- 不调整名称文本截断处现有 `title` 行为（例如代理名 / Prompt Cache Key 文本）。

## 范围（Scope）

### In scope

- `web/src/components/ui/` 新增共享 inline chart tooltip 原语与交互 hook。
- `web/src/components/ForwardProxyLiveTable.tsx`：接入请求量柱图与权重趋势图的统一 tooltip、高亮与无障碍文案。
- `web/src/components/PromptCacheConversationTable.tsx`：接入 Token 累计 sparkline 的统一 tooltip、高亮与无障碍文案。
- `web/src/i18n/translations.ts`：补齐 tooltip 标签、交互提示与图表 aria 文案。
- `web/src/components/*.test.tsx` 与新增交互测试：覆盖 SSR 回归、无原生 tooltip、pointer/tap/keyboard 交互。
- Storybook：为两个表格组件补充高密度边缘 tooltip 场景。

### Out of scope

- Rust 后端、SQLite、SSE hook 与轮询策略改动。
- Live 页整体布局重构。
- 新增 E2E 框架或第三方 tooltip 组件库。

## 验收标准（Acceptance Criteria）

- Given Live 代理请求量柱图，When 用户悬停某个桶，Then 指针旁显示统一样式 tooltip，内容包含时间范围、成功数、失败数、总请求数，且当前桶高亮。
- Given Live 代理权重趋势图，When 用户悬停或键盘切换某个点，Then 指针/焦点旁显示时间范围、样本、最小、最大、平均、末值，且当前点与垂直导引线高亮。
- Given Prompt Cache Token 累计 sparkline，When 用户悬停或点按某段，Then 显示时间、状态、本次 Tokens、累计 Tokens，并高亮当前段与参考线。
- Given 图表获得键盘焦点，When 使用 `ArrowLeft` / `ArrowRight` / `Home` / `End` / `Escape`，Then 可切换或关闭 tooltip，且整张图只占用一个 tab stop。
- Given 触屏点击某个数据点，When tooltip 打开后再次点击同点或点击外部，Then tooltip 关闭；不存在浏览器原生 tooltip。
- Given 运行 `cd web && npm run test && npm run build`，When 检查相关组件，Then 测试与构建通过。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新建 spec 并登记到 `docs/specs/README.md`。
- [x] M2: 新增共享 inline chart tooltip 原语、定位逻辑与 pointer/tap/keyboard 交互 hook。
- [x] M3: ForwardProxyLiveTable 与 PromptCacheConversationTable 接入统一 tooltip / 高亮 / aria 文案。
- [x] M4: 补齐 Vitest SSR + jsdom 交互覆盖，并更新 Storybook 场景。
- [ ] M5: 完成本地验证、浏览器复核、fast-track PR 与 spec 同步。

## 进度备注

- 默认 tooltip 锚点相对指针偏移 `12px`，在容器内自动翻转并做边缘 clamp，避免在表格右缘被裁切。
- 键盘默认激活“最近一个有效数据点”；若整图无有效点，则回退到最后一个桶/点并显示零值或平线详情。
- 请求量柱图 tooltip 仅新增 `总请求数 = success + failure` 的前端展示，不引入新的后端字段。
- 交互测试使用 Vitest + jsdom 文件级环境，不引入新的测试框架。
- 本地已通过 `cd web && npm run lint && npm run test && npm run build`。
- 已用 Storybook mock + chrome-devtools 复核 hover / keyboard / tap 三套交互，未见原生 tooltip 残留。

## 参考

- `web/src/components/ForwardProxyLiveTable.tsx`
- `web/src/components/PromptCacheConversationTable.tsx`
- `web/src/components/UsageCalendar.tsx`
- `docs/specs/c58kc-live-forward-proxy-table/SPEC.md`
- `docs/specs/t7m4h-live-proxy-weight-trend/SPEC.md`
- `docs/specs/4kkpp-live-prompt-cache-conversations/SPEC.md`
