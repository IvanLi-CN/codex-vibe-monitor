# Live 对话统计（按 Prompt Cache Key）— 无统计表方案（#4kkpp）

## 状态

- Status: 已完成
- Created: 2026-03-03
- Last: 2026-03-03

## 背景 / 问题陈述

- Live 页已有请求级与代理级视图，但缺少“按 Prompt Cache Key 归并后的对话级”观察能力。
- 线上 24h 请求量已达万级，且请求普遍携带 `promptCacheKey`，需要低复杂度且可持续的对话统计能力。
- 直接引入会话统计表虽可读性能稳定，但会显著增加双写一致性与回填维护成本。

## 目标 / 非目标

### Goals

- 在 Live 页新增“最近 24 小时对话统计”区域，按 `promptCacheKey` 聚合展示。
- 对话集合取最近 24 小时活跃 key；累计指标（请求数 / tokens / 成本）按全历史计算。
- 提供 24h 请求累计 Token step 图（成功段绿色、失败段红色，区间遵循 `[请求, 下一个请求)`）。
- 不建统计表，采用“表达式索引 + 5 秒轻缓存 + 前端节流刷新”。

### Non-goals

- 不新增 `conversation_stats` 或触发器/双写管道。
- 不修改 SSE 协议格式与事件类型。
- 不改 Dashboard/Stats 页面信息架构。

## 范围（Scope）

### In scope

- 后端新增 `GET /api/stats/prompt-cache-conversations`。
- `codex_invocations` 增加 `promptCacheKey` 表达式复合索引（`IF NOT EXISTS`）。
- 后端新增 5s 进程内轻缓存（按 `limit=20/50/100` 分桶）与同 key in-flight 复用。
- 前端新增 Live 对话统计区块、组件、hook 与中英文文案。
- 补齐 Rust/Vitest 验证。

### Out of scope

- 不做跨实例共享缓存。
- 不引入新的数据库表结构迁移。

## 接口契约（Interfaces & Contracts）

- `GET /api/stats/prompt-cache-conversations?limit=<20|50|100>`
  - `rangeStart` / `rangeEnd`
  - `conversations[]`
    - `promptCacheKey`
    - `requestCount`
    - `totalTokens`
    - `totalCost`
    - `createdAt`
    - `lastActivityAt`
    - `last24hRequests[]`
      - `occurredAt`
      - `status`
      - `isSuccess`
      - `requestTokens`
      - `cumulativeTokens`

## 验收标准（Acceptance Criteria）

- Given 最近 24h 有同 key 多次请求，When 打开 Live 页，Then 显示 1 条对话并给出正确累计值。
- Given 对话历史跨越 24h，When 查询接口，Then `requestCount/totalTokens/totalCost/createdAt/lastActivityAt` 为全历史口径。
- Given 请求缺失 `promptCacheKey`，When 查询接口，Then 不进入对话列表。
- Given step 图渲染，When 检查颜色区间，Then 每段对应 `[请求_i, 请求_{i+1})` 且成功绿失败红。
- Given `limit` 切换 `20/50/100`，When 页面刷新，Then 列表数量同步变化。
- Given 高频 SSE 事件，When 页面持续停留，Then 查询频率受节流与缓存控制。

## 质量门槛（Quality Gates）

- `cargo fmt`
- `cargo check`
- `cargo test`
- `cd web && npm run test`
- `cd web && npm run build`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 后端新增对话统计接口与查询聚合。
- [x] M2: `promptCacheKey` 表达式索引 + 5s 轻缓存 + singleflight。
- [x] M3: 前端 Live 区块、组件、hook、i18n 完成。
- [x] M4: Rust/Vitest/Build 全量通过。
- [x] M5: fast-track 交付（push + PR + checks + review-loop 收敛）。

## 风险 / 假设

- 风险：表达式索引未命中时可能回退全表 JSON 解析，导致高并发抖动。
- 风险：进程内缓存为实例级，横向扩展后实例间结果有秒级时差。
- 假设：线上请求稳定携带 `promptCacheKey`，缺失占比可忽略。

## 变更记录（Change log）

- 2026-03-03: 新建规格，冻结“无统计表 + 轻缓存”实现策略。
- 2026-03-03: 完成后端聚合接口、表达式索引、5s 轻缓存与前端 Live 对话统计区块，质量门槛（cargo + web test/build）通过并进入 fast-track 交付链路。

## 参考

- `src/main.rs`
- `web/src/pages/Live.tsx`
- `web/src/lib/api.ts`
- `web/src/hooks/usePromptCacheConversations.ts`
- `web/src/components/PromptCacheConversationTable.tsx`
