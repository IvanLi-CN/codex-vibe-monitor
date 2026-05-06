# Live 代理运行态：新增 24h 权重趋势列与断点适配（#t7m4h）

## 状态

- Status: 已完成（5/5）
- Created: 2026-03-02
- Last: 2026-03-02

## 背景 / 问题陈述

- `Live` 页代理运行态当前只有“近 24 小时请求量（成功/失败）”，缺少权重变化视角。
- 现场排障时无法直接判断调度权重在过去 24 小时内的恢复/惩罚轨迹。
- 当前请求量列底部重复展示成功/失败文案，信息密度偏高。

## 目标 / 非目标

### Goals

- 在 `/api/stats/forward-proxy` 返回每个节点固定 24 桶的 `weight24h`（1 小时粒度）。
- 在 Live 代理表新增“近 24 小时权重变化”独立列，保留原请求量图。
- 去掉请求量图底部“成功/失败”文字，仅保留节点摘要处成功/失败总量。
- 仅在 Live 页完成断点适配，不改全局容器上限（保留 1200）。

### Non-goals

- 不修改 forward proxy 权重算法（v1/v2）及路由策略。
- 不改 Dashboard/Stats/Settings 页面布局与交互。
- 不引入新的前端图表依赖。

## 范围（Scope）

### In scope

- 后端新增 `forward_proxy_weight_hourly` 统计桶存储与查询。
- `/api/stats/forward-proxy` 节点响应新增 `weight24h`。
- Live 页代理表新增权重趋势列、更新列宽断点策略。
- 补齐 Rust 与 Vitest 覆盖。

### Out of scope

- 权重历史 retention 策略扩展（例如清理任务）。
- Settings 页内权重趋势可视化。

## 验收标准（Acceptance Criteria）

- Given 任意节点，When 查询 `/api/stats/forward-proxy`，Then `weight24h` 固定返回 24 个桶。
- Given 桶内无采样，When 返回 `weight24h`，Then 使用 carry-forward 权重且 `sampleCount=0`。
- Given 全区间无历史，When 返回 `weight24h`，Then 以当前 runtime.weight 生成平线。
- Given Live 表格渲染，When 查看请求量列，Then 不再出现底部“成功/失败”文字。
- Given Live 表格渲染，When 查看新列，Then 可见“近 24 小时权重变化”趋势图。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 后端存储层增加 `forward_proxy_weight_hourly` 表与查询函数。
- [x] M2: `/api/stats/forward-proxy` 增加 `weight24h` 并实现固定 24 桶补齐。
- [x] M3: 前端 API 类型/normalize 支持 `weight24h`。
- [x] M4: Live 代理表新增权重趋势列并完成断点布局调整。
- [x] M5: Rust + Vitest 校验通过并进入 fast-track PR 流程。

## 变更记录（Change log）

- 2026-03-02: 新建规格，冻结本轮实现范围与验收口径。
- 2026-03-02: 完成后端 `weight24h` 聚合与前端表格改版，补齐 Rust/Vitest 校验，进入 fast-track PR 阶段。
- 2026-03-02: 收敛 review-loop，补齐权重桶并发写入顺序保护、i18n/可访问性文案，PR #83 checks 全绿。

## 参考（References）

- `src/main.rs`
- `web/src/components/ForwardProxyLiveTable.tsx`
- `web/src/pages/Live.tsx`
- `web/src/lib/api.ts`
