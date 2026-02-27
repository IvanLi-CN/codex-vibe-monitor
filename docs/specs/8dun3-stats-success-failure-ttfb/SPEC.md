# 统计页成功/失败图增加首字耗时折线与悬浮统计（#8dun3）

## 状态

- Status: 已完成
- Created: 2026-02-27
- Last: 2026-02-27

## 背景 / 问题陈述

- 统计页“成功/失败次数”图当前只展示数量分布，无法直接观察响应性能变化。
- 现有悬浮提示只显示成功/失败次数，缺少成功率与首字耗时统计，诊断效率较低。
- 现有前端 SSE 本地增量合并无法正确维护百分位指标，直接在前端做 P95/P99 会出现失真。

## 目标 / 非目标

### Goals

- 在成功/失败图中增加“首字耗时均值”折线（按当前 bucket 维度）。
- tooltip 增加“成功率、首字耗时均值、首字耗时 P95”（仅当前 bucket）。
- 后端 `/api/stats/timeseries` 输出兼容扩展字段：`firstByteSampleCount`、`firstByteAvgMs`、`firstByteP95Ms`。
- Stats 页切换为服务端聚合优先，SSE records 到达时触发节流重拉以保证百分位准确性。

### Non-goals

- 不在本次图表中加入 P99。
- 不改动“趋势”图（Tokens/Cost/次数）的视觉与数据口径。
- 不引入新的性能页面、筛选控件或数据库 schema 变更。

## 范围（Scope）

### In scope

- `src/main.rs`（timeseries 聚合字段与测试）
- `web/src/lib/api.ts`（TimeseriesPoint 类型扩展）
- `web/src/hooks/useTimeseries.ts`（服务端聚合优先模式）
- `web/src/pages/Stats.tsx`（Stats 页接入）
- `web/src/components/SuccessFailureChart.tsx`（柱线组合图 + tooltip）
- `web/src/components/SuccessFailureChart.test.tsx`
- `web/src/hooks/useTimeseries.test.ts`
- `web/src/i18n/translations.ts`
- `docs/specs/README.md`

### Out of scope

- `src/` 其他 API 接口语义调整。
- `web` 其他图表组件重构。

## 接口与数据口径

- 后端 `TimeseriesPoint` 新增字段：
  - `firstByteSampleCount: i64`
  - `firstByteAvgMs: Option<f64>`
  - `firstByteP95Ms: Option<f64>`
- 样本筛选规则：仅 `status == success` 且 `t_upstream_ttfb_ms > 0` 且有限值计入。
- 样本为空时返回：`firstByteSampleCount=0`，`firstByteAvgMs=null`，`firstByteP95Ms=null`。

## 验收标准（Acceptance Criteria）

- Given 成功 bucket 含有效首字耗时样本，When 请求 `/api/stats/timeseries`，Then 新增字段为有效数值且口径正确。
- Given bucket 无有效首字耗时样本，When 请求 `/api/stats/timeseries`，Then `firstByteSampleCount=0` 且均值/P95 为 `null`。
- Given 打开统计页成功/失败图，When 悬浮任意 bucket，Then tooltip 显示“失败、成功、成功率、首字耗时均值、首字耗时 P95”共 5 项。
- Given bucket 无有效首字耗时样本，When 悬浮该 bucket，Then tooltip 对应字段显示 `—`，且不出现 `NaN/Infinity`。
- Given 开启 Stats SSE 更新，When records 到达，Then Stats 的 timeseries 通过节流重拉维持服务端聚合准确性。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 创建 spec 并登记到 `docs/specs/README.md`。
- [x] M2: 后端 timeseries 新增首字耗时样本聚合字段（普通桶/按天桶）。
- [x] M3: 前端类型、hook、Stats 页接入服务端聚合优先策略。
- [x] M4: 成功/失败图升级为柱线图并完成 tooltip 字段扩展。
- [x] M5: 补充后端与前端测试。
- [x] M6: 完成验证矩阵（cargo/web test/build）并交付快车道 PR。

## 进度备注

- 后端 `/api/stats/timeseries` 已新增首字耗时聚合字段：`firstByteSampleCount`、`firstByteAvgMs`、`firstByteP95Ms`，并在普通/按天 bucket 统一口径。
- Stats 页已切换为 `preferServerAggregation=true`，records SSE 到达后优先节流重拉，避免前端本地增量造成 P95 失真。
- 成功/失败图已升级为“堆叠柱 + 首字耗时均值折线”，tooltip 固定展示 5 项（失败、成功、成功率、均值、P95）。
- 已新增测试：`timeseries_*first_byte*`（Rust）与 `SuccessFailureChart.test.tsx`、`useTimeseries.test.ts`（Vitest）。
- 验证通过：`cargo fmt --check`、`cargo check`、`cargo test`、`cd web && npm run test`、`cd web && npm run build`。

## 风险与回滚

- 风险：SSE 本地增量与服务端重拉并存时可能出现短暂闪动。
- 缓解：保持节流重拉与 silent loading，避免大范围 loading 抖动。
- 回滚：若出现性能或显示问题，可先隐藏折线与新 tooltip 字段，保留后端字段兼容不影响旧消费者。
