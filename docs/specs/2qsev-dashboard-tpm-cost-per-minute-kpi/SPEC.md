# Dashboard 今日 KPI 上下文统计卡片（#2qsev）

## 背景 / 问题陈述

- Dashboard 今日 KPI 仍以累计总量为主，首个 tile 展示“调用总数”，缺少更贴近当前负载的实时速率视角。
- 主人要求把首个统计替换成当前每分钟 Token 数，并新增当前每分钟金额，同时继续保留成功/失败/总成本/总 Tokens 的累计视角。
- today 面板已经拉取 `useTimeseries('today', { bucket: '1m' })`，可以直接前端派生速率；如果继续只展示累计值，今天面板和分钟级图表之间会割裂，难以快速判断当前吞吐。
- 今日 KPI 需要从“单一大数字卡片”升级为“一个主要信息 + 两个辅助信息”的扫描模型，补充失败率、缓存命中、前 7 完整日均值、与昨日口径比较，以及第三个卡片后的实时并行对话数。

## 目标 / 非目标

### Goals

- 将今日 KPI 的首个 tile 改为 `TPM`，并新增 `消费速率` / `Spend rate`。
- 速率口径固定为最近 5 分钟内的活跃尾段均值：当前进行中分钟参与计算，首个非零 token/cost 桶之前的前置空闲时间不参与分母，首个活动桶之后的空闲时间继续参与分母。
- 每个统计卡片固定表达一个主要信息和两个辅助信息。
- 合并成功和失败卡片：主要显示成功数，辅助显示失败数和失败率。
- 在成功卡片后增加并行对话卡片：主要显示实时并行对话数，辅助显示与昨日平均并行数比较和今日日均并行数。
- `TPM` 与 `消费速率` 辅助显示今日工作分钟日均值，以及与昨日日均的百分比差异。工作分钟日均值只统计有调用的分钟切片。
- `总成本` 改名为 `今日成本`，辅助显示前 7 个完整自然日的每日均值，以及与昨日完整日成本的百分比差异。
- `总 Tokens` 改名为 `今日 Tokens`，辅助显示缓存命中比率，以及与昨日完整日 Tokens 的百分比差异。
- 与昨日比较的百分比必须用正负颜色区分。
- 数据卡片标题可点击 / 聚焦 / 悬停打开 tooltip，解释字段含义与速率算法。
- summary 成功但 timeseries 尚未可用时，只让两个速率 tile 进入 skeleton / `—` 降级，其余累计 tile 保持可读。
- `TodayStatsOverview` 升级成 6 个等权 tile，并在 Storybook `desktop1440` 下保持单行。
- 生成并归档 Storybook 视觉证据，供快车道 PR merge-ready 使用。

### Non-goals

- 不修改 SQLite schema、SSE 协议或历史 rollup 存储结构。
- 不改变 Dashboard `24 小时 / 7 日 / 历史` 切换、metric toggle 和记忆行为。
- 不把 `5m avg` 伪装成严格最近 1 分钟瞬时值。

## 范围（Scope）

### In scope

- `web/src/components/DashboardActivityOverview.tsx`：新增 today 速率派生层并把 snapshot 传入 KPI 组件。
- `web/src/components/TodayStatsOverview.tsx`：重排为 6 tile，并支持速率 tile 独立 loading / unavailable。
- `web/src/components/dashboardKpiComparisons.ts`：集中派生工作分钟日均、缓存命中、失败率、昨日比较与并行对话快照。
- `src/api/slices/**`、`src/stats/mod.rs`：为 summary 增加 `previous7d` 窗口，并让分钟级 timeseries 暴露 `cacheInputTokens`。
- `web/src/hooks/useParallelWorkStats.ts`：支持禁用账号范围下的全局并行对话请求。
- `web/src/components/TodayStatsOverview.stories.tsx`、`web/src/components/DashboardActivityOverview.stories.tsx`：补齐 populated / loading / error / zero-rate 等 Storybook 场景。
- `web/src/components/*.test.tsx`、`web/src/pages/Dashboard.test.tsx`：补齐速率算法、KPI 渲染与 partial fallback 回归。
- `web/src/i18n/translations.ts`：新增上下文 KPI 相关文案。

### Out of scope

- `src/` 下任意后端实现。
- 新增后端 rate summary API。
- Dashboard 其它卡片或图表布局的额外重构。

## 需求（Requirements）

### MUST

- 最近 5 分钟均值按活跃尾段时间加权计算。
- trailing window 内首个非零 token/cost 桶之前的前置 0 时间不参与分母；首个活动桶之后的缺失或 0 时间继续参与分母，避免一有请求就产生尖峰。
- 今日窗口内活动尾段少于 5 分钟时，按实际活动尾段 elapsed minutes 求均值；若无活动，显示数值 `0`。
- 当前进行中分钟参与显示速率；由于 timeseries 为 1m 桶，首个活动秒级起点按该桶 `bucketStart` 近似。
- 今日工作分钟日均值按 `今日总量 / totalCount > 0 的 1m 切片数` 计算；没有工作分钟时显示 `—`。
- 昨日比较采用昨日同口径完整自然日：TPM / 消费速率使用昨日工作分钟日均，成本 / Tokens 使用昨日完整日总量，并行对话使用昨日平均并行数。
- 前 7 日每日均值采用今天之前的 7 个完整自然日，不包含今天。
- `今日 Tokens` 的缓存命中比率使用 today timeseries 中的 `cacheInputTokens / totalTokens` 派生；无 tokens 时显示 `0%`。
- summary error 时保持整个 today overview 现有 alert 语义。
- timeseries error 时，仅两个速率 tile 显示 `—`。

### SHOULD

- 速率 helper 独立成可测试模块，避免把计算逻辑塞进组件 JSX。
- Storybook 直接展示 6-tile 单行桌面态和 partial fallback 态。

### COULD

- 速率 snapshot 携带 `windowMinutes`，便于后续 tooltip/文案复用。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- Dashboard 今日视图加载成功后，KPI 行显示：`TPM`、`消费速率` / `Spend rate`、`成功`、`并行对话`、`今日成本`、`今日 Tokens`。
- `TPM` 与 `消费速率` 来自 today 1 分钟时序：以同自然日内较新的 `now` / `rangeEnd` 为 anchor，取最近 5 分钟内的 points，找到最早的非零 token/cost bucket，并用 `anchor - activeStart` 作为分母。
- `TPM` 与 `消费速率` 辅助信息分别显示今日工作分钟日均值和较昨日工作分钟日均值的百分比差异。
- `成功` 卡片以成功数为主要信息，辅助展示失败数和失败率。
- `并行对话` 卡片以最新分钟并行数为主要信息，辅助展示较昨日平均并行数的百分比差异和今日平均并行数。
- `今日成本` 卡片以今日累计成本为主要信息，辅助展示前 7 完整日每日均值和较昨日完整日成本的百分比差异。
- `今日 Tokens` 卡片以今日累计 Tokens 为主要信息，辅助展示缓存命中比率和较昨日完整日 Tokens 的百分比差异。
- 如果最近 5 分钟内前三分半都是 0、后 1.5 分钟有数据，则使用后 1.5 分钟总量除以 `1.5`。
- 如果首个活动桶之后存在缺失分钟或 0 值分钟，这段时间继续计入分母。

### Edge cases / errors

- 今日最近 5 分钟无 token/cost 活动时：速率显示 `0`，不是 `—`。
- timeseries 正在加载且 summary 已成功：速率 tile skeleton，其余 4 个 tile 正常显示。
- timeseries 加载失败且 summary 已成功：速率 tile 显示 `—`，其余 4 个 tile 正常显示。
- summary 失败时：整体显示现有 alert，不做混合态拼接。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name）                      | 类型（Kind）        | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers）                                 | 备注（Notes）                                  |
| --------------------------------- | ------------------- | ------------- | -------------- | ------------------------ | --------------- | --------------------------------------------------- | ---------------------------------------------- |
| Dashboard today rate snapshot     | ui-component-prop   | internal      | Modify         | None                     | web/dashboard   | `DashboardActivityOverview` -> `TodayStatsOverview` | 前端本地派生 5m 速率                           |
| Dashboard KPI comparison snapshot | ui-helper           | internal      | Add            | None                     | web/dashboard   | `DashboardActivityOverview`、`TodayStatsOverview`   | 派生工作分钟日均、失败率、缓存命中与比较百分比 |
| Summary `previous7d`              | http-query          | backend-api   | Add            | None                     | backend/stats   | Dashboard summary hooks                             | 返回今天之前 7 个完整自然日汇总                |
| Timeseries `cacheInputTokens`     | http-response-field | backend-api   | Add            | None                     | backend/stats   | Dashboard today KPI                                 | 用于缓存命中比率                               |
| Parallel work KPI snapshot        | ui-component-prop   | internal      | Add            | None                     | web/dashboard   | `DashboardActivityOverview` -> `TodayStatsOverview` | 全局并行对话统计，账号范围下禁用               |

### 契约文档（按 Kind 拆分）

- None

## 验收标准（Acceptance Criteria）

- Given 最近 5 分钟内前三分半都是 0、后 1.5 分钟累计 `1500 tokens / US$0.15`，When 打开 Dashboard 今日视图，Then `TPM = 1000` 且 `消费速率 = US$0.10`。
- Given trailing 5 分钟内首个活动桶之后缺少某些分钟桶，When 计算速率，Then 缺失时间继续计入分母。
- Given 当前自然分钟尚未完成但已有数据，When 计算速率，Then 当前分钟参与 displayed rate，分母使用实际 elapsed minutes。
- Given 最近 5 分钟没有任何 token/cost 活动，When timeseries 已返回，Then 两个速率 tile 显示 `0`。
- Given 点击或聚焦任一 KPI 标题，When tooltip 打开，Then 能看到该字段的本地化说明。
- Given summary 成功但 timeseries 正在加载，When 渲染 today KPI，Then 只有两个速率 tile 显示 skeleton。
- Given summary 成功但 timeseries 失败，When 渲染 today KPI，Then 只有两个速率 tile 显示 `—`。
- Given summary 失败，When 渲染 today KPI，Then 保持现有整块 alert 语义。
- Given Storybook `desktop1440` 视口，When 查看 today KPI，Then 6 个 tile 单行展示且无横向溢出。
- Given 今日统计加载成功，When 查看 KPI 行，Then 卡片顺序为 `TPM`、`消费速率`、`成功`、`并行对话`、`今日成本`、`今日 Tokens`。
- Given 今日有成功和失败调用，When 查看成功卡片，Then 主信息是成功数，辅助信息是失败数和失败率。
- Given 今日并行对话统计可用，When 查看并行对话卡片，Then 主信息是最新并行数，辅助信息是较昨日和日均。
- Given 今日有 cache input tokens，When 查看今日 Tokens 卡片，Then 辅助信息显示缓存命中比率。
- Given 与昨日比较有正负变化，When 查看 KPI 辅助信息，Then 正值与负值使用不同颜色。

## 实现前置条件（Definition of Ready / Preconditions）

- 速率口径、降级态与文案已冻结。
- 不新增后端契约这一边界已确认。
- Storybook 仍是本次视觉证据的主源。

### UI / Storybook (if applicable)

- Stories to add/update: `TodayStatsOverview.stories.tsx`、`DashboardActivityOverview.stories.tsx`
- Docs pages / state galleries to add/update: 复用 autodocs + state gallery
- `play` / interaction coverage to add/update: `DashboardActivityOverview.stories.tsx` today 视图断言速率 tile
- Visual regression baseline changes (if any): 以 spec 内 `## Visual Evidence` 为准

### Quality checks

- `cargo fmt --check`
- `cargo check`
- `cargo test parse_summary_window_accepts_previous_seven_full_days_window`
- `cargo test previous_full_days_range_ends_at_current_local_midnight`
- `cd web && bun run test`
- `cd web && bun run build`
- `cd web && bun run test-storybook`
- `cd web && bun run build-storybook`

## 计划资产（Plan assets）

- Directory: `docs/specs/2qsev-dashboard-tpm-cost-per-minute-kpi/assets/`
- In-plan references: `![...](./assets/<file>.png)`
- Visual evidence source: maintain `## Visual Evidence` in this spec when owner-facing or PR-facing screenshots are needed.

## Visual Evidence

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: browser-viewport
  sensitive_exclusion: N/A
  submission_gate: chat-reviewed
  story_id_or_title: `dashboard-todaystatsoverview--populated`
  state: populated with KPI title tooltip open
  evidence_note: 证明 KPI 标题可点击打开字段说明 tooltip，TPM 文案明确说明最近 5 分钟活跃尾段均值；本次证据只回传聊天快照，不新增提交截图文件。
  PR: no-image

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  sensitive_exclusion: N/A
  submission_gate: approved
  story_id_or_title: `dashboard-todaystatsoverview--desktop-single-row`
  state: desktop single row
  evidence_note: 证明 `desktop1440` 下 6 个 KPI tile 保持单行，且顺序为 `TPM`、`消费速率`、`成功`、`并行对话`、`今日成本`、`今日 Tokens`；辅助信息包含日均、较昨日、失败率、7 日均与缓存命中。
  PR: include
  image:
  ![Today KPI desktop single row](./assets/today-kpi-desktop-single-row.png)

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  sensitive_exclusion: N/A
  submission_gate: approved
  story_id_or_title: `dashboard-todaystatsoverview--state-gallery`
  scenario: state gallery
  evidence_note: 证明 populated、rate loading、rate unavailable、summary loading 与 summary error 的分态降级行为；速率不可用时其余上下文 KPI 仍保持可读。
  PR: include
  image:
  ![Today KPI state gallery](./assets/today-kpi-state-gallery.png)

## 资产晋升（Asset promotion）

- None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: today 1 分钟时序速率 helper 落地并接入 Dashboard today KPI。
- [x] M2: `TodayStatsOverview` 升级为 6 tile，支持速率 tile 独立 loading / unavailable。
- [x] M3: Vitest 与 Storybook 场景覆盖补齐。
- [x] M4: 视觉证据归档并推进到 PR merge-ready。

## 方案概述（Approach, high-level）

- 继续复用 today 视图现有的 `useSummary + useTimeseries` 双源模式，只在前端增加一层轻量 rate snapshot 归一化。
- 额外请求 `yesterday` 与 `previous7d` summary、today/yesterday 1m timeseries 以及全局并行对话统计，集中在前端派生比较型辅助信息。
- 通过把 summary 与 rate 状态分离，让“累计 KPI”与“速率 KPI”按不同可用性降级，避免一边失败拖垮整排卡片。
- Storybook 继续作为最稳定的视觉证据来源，避免用真实页面随机数据截图。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：`useTimeseries(today)` 只提供 1m 桶，无法恢复首个非零桶内的真实秒级活动起点；本轮按 `bucketStart` 近似并通过 tooltip 明确口径。
- 风险：6 tile 在较窄桌面宽度下可能压缩数值，需要继续依赖 `AdaptiveMetricValue` 的 compact fallback。
- 假设：`desktop1440` 是本次“单行 KPI”主要验收视口。

## 参考（References）

- `docs/specs/7s4kw-dashboard-usage-activity-overview/SPEC.md`
- `docs/specs/r99mz-dashboard-today-activity-overview/SPEC.md`
