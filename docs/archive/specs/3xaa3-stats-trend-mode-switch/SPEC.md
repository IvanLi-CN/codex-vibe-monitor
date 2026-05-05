# 统计页趋势图按点数切换柱状 / 累积面积模式（#3xaa3）

## 状态

- Status: 已完成
- Created: 2026-03-20
- Last: 2026-03-20

## 背景 / 问题陈述

- 统计页顶部“趋势”图当前按 `48` 个点做视觉分流：点少时展示按桶柱状图，点多时展示面积图。
- 主人要求更直接的阅读规则：当数据点 `<= 7` 时继续看离散柱状图；超过 `7` 个点后改看累计走势，而不是继续看逐桶离散值。
- 如果只切图形不切口径，面积图会继续展示 bucket 原值，无法表达“更长时间窗口下总量如何一路累积”的趋势。

## 目标 / 非目标

### Goals

- 将 Stats 页顶部 `TimeseriesChart` 的模式阈值改为 `7` 个点。
- 当点数 `<= 7` 时保留现有逐桶柱状图口径。
- 当点数 `> 7` 时，对 `totalTokens`、`totalCount`、`totalCost` 计算 running sum，并以面积图展示累计走势。
- 补充独立的前端回归测试，锁住 `7/8` 边界、累计数学口径，以及组件实际渲染模式。
- 用 101 的生产数据做一致性 SQLite 快照，在共享测试机实际启动预览并核验两种图表模式。

### Non-goals

- 不改动统计页下方“成功/失败次数”图。
- 不修改 `/api/stats/timeseries` 的协议字段、SSE 语义或 SQLite schema。
- 不复制 `proxy_raw_payloads`、archives 或其它非本次预览必需数据。

## 范围（Scope）

### In scope

- `web/src/components/TimeseriesChart.tsx`
- `web/src/components/timeseriesChartModel.ts`
- `web/src/components/timeseriesChartModel.test.ts`
- `web/src/components/TimeseriesChart.test.tsx`
- `web/src/pages/Stats.test.tsx`
- `web/src/pages/Stats.bucket-fallback.test.tsx`
- `docs/specs/README.md`
- 当前 spec 与验收记录

### Out of scope

- `web/src/components/SuccessFailureChart.tsx`
- Rust 后端聚合逻辑、`web/src/lib/api.ts` 类型定义、数据库迁移
- Storybook 资产、PR 内嵌截图文件

## 接口与展示口径

- 公共 HTTP API、SSE 与持久化 schema 保持不变。
- 组件内部新增两条前端可观测元信息：
  - `data-chart-kind="stats-timeseries-trend"`
  - `data-chart-mode="bucket-bar" | "cumulative-area"`
- 累积模式的计算规则：
  - 输入顺序沿用 `points` 当前时间顺序，不做重新排序。
  - `totalTokens` / `totalCount` / `totalCost` 都按前缀和计算。
  - legend 与 tooltip 仍沿用现有三项名称，不新增“累计”专用字段。

## 验收标准（Acceptance Criteria）

- Given `TimeseriesChart` 输入点数为 `7`，When 渲染趋势图，Then 使用逐桶柱状图，并保留原始 bucket 数值。
- Given `TimeseriesChart` 输入点数为 `8` 或更多，When 渲染趋势图，Then 使用面积图，并展示 `totalTokens` / `totalCount` / `totalCost` 的累计值。
- Given 累积模式启用，When 检查任意后续点，Then 三条序列都等于从首点到当前点的 running sum，且非递减。
- Given Stats 页现有范围 / 粒度选择逻辑，When 切换范围或后端返回 bucket fallback，Then 页面行为继续兼容，不因本次趋势图切换回归。
- Given 使用 101 上 `ai-codex-vibe-monitor-data` 的一致性 SQLite 快照在共享测试机启动预览，When 打开 Stats 页并切换到一组 `<= 7` 点配置与一组 `> 7` 点配置，Then 可以实际看到柱状图 / 累积面积图两种模式。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: `cd web && bunx vitest run src/components/timeseriesChartModel.test.ts src/components/TimeseriesChart.test.tsx src/pages/Stats.test.tsx src/pages/Stats.bucket-fallback.test.tsx`

### Quality checks

- TypeScript build: `cd web && bunx tsc -b`
- Production build: `cd web && bun run build`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 建立 focused spec 并登记到 `docs/specs/README.md`。
- [x] M2: 将趋势图模式判定与累计数据转换抽到独立 model，阈值切换为 `7`。
- [x] M3: 为趋势图组件补齐 `7/8` 边界与累计口径回归测试。
- [x] M4: 完成本地前端验证（Vitest / TypeScript / build）。
- [x] M5: 基于 101 一致性快照在共享测试机启动真实预览并完成界面核验。
- [x] M6: 完成 fast-track 提交、PR、checks 与 review-loop 收敛到 merge-ready。

## 方案概述（Approach, high-level）

- 新增 `timeseriesChartModel` 作为趋势图唯一的数据整形入口，统一负责：
  - 点数阈值判定；
  - 标签格式化；
  - 累计模式的 running-sum 转换。
- `TimeseriesChart` 只消费 model 产出的 `chartMode + chartData`，避免把视觉分支和数学口径散落在 JSX 中。
- 组件根节点附加 `data-chart-kind` / `data-chart-mode`，让 SSR/mock 测试可以直接证明渲染分支。
- 真实验收使用 101 主库的 `.backup` 快照，只同步 `codex_vibe_monitor.db` 到共享测试机隔离 run，再通过 Dockerfile 启动预览容器。

## 风险 / 假设 / 回滚

- 风险：`codex_vibe_monitor.db` 较大，快照与传输耗时可能明显高于普通 shared-testbox smoke。
- 缓解：使用 host `sqlite3 .backup` 获取一致性副本，并只同步单个 DB 文件。
- 风险：本 session 未暴露 `chrome-devtools` MCP 时，浏览器验收需要改用本地可用的无头浏览器或退化为 HTTP/DOM 证据。
- 假设：`/api/stats/timeseries` 返回点位已按时间升序排列，前端无需再排序。
- 回滚：若累计面积图引发解读问题，可仅回退 `timeseriesChartModel` 的阈值与 running-sum 分支，不影响后端接口或其它图表。

## 变更记录（Change log）

- 2026-03-20: 创建 spec，冻结“Stats 顶部趋势图按 `<=7` / `>7` 切换柱状与累积面积”的实现边界。
