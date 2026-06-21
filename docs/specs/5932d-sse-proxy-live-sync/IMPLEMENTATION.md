# SSE 驱动的请求记录与统计实时更新 - Implementation

## Current State

- Canonical spec: `docs/specs/5932d-sse-proxy-live-sync/SPEC.md`
- Implementation summary: 已完成
- Dashboard realtime consumption now separates SSE-fast KPI commits from 5s HTTP/chart reconcile budgets. Working conversations batch visible SSE patches for 1s and throttle head/snapshot reconcile to 5s. `/api/stats/parallel-work` keeps its response schema while supporting ETag / 304 conditional fetches.

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-02-25
- Last: 2026-02-25

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- 运行并通过与改动直接相关的 Rust 测试（覆盖代理落库后广播路径）。
- 运行并通过前端构建或测试校验（至少一种自动化验证）。

## Migrated Implementation Sections

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 抽取落库后广播 helper，并改造 `persist_proxy_capture_record` 返回语义支持“是否新插入”。
- [x] M2: 替换代理链路 5 处落库调用点为统一 helper。
- [x] M3: 前端 `useInvocationStream` 增加 SSE open 后静默回源补齐。
- [x] M4: 完成验证、提交、PR、checks 与 review-loop 收敛（fast-track）。
- [x] M5: Dashboard realtime consumers split visible patch, KPI, chart commit, head reconcile, and parallel-work conditional-fetch budgets.
- [x] M6: 活动调用记录列表统一接入 `records` SSE：`Live`、`/records` 与账号详情抽屉 records tab 现在共用一套记录过滤、去重、终态优选与 SSE open 静默回源逻辑。

## 2026-06-21 Follow-up

- 新增 `web/src/lib/invocationRecordsLive.ts`，把活动记录窗口的过滤、排序、去重与“更完整终态记录优选”抽成共享工具，避免 `Live`、账号详情抽屉和 `/records` 页各维护一套实时合并语义。
- 新增 `web/src/hooks/useInvocationRecordsRealtime.ts`，统一负责 `records` SSE 订阅、已命中窗口内的可见记录合并，以及 SSE `open` 后静默 reconcile。
- 账号详情抽屉 records tab 不再只做一次性 `fetchInvocationRecords(...)`；它现在按 `upstreamAccountId + limit + tab/open lifecycle` 受控订阅 SSE，并在连接恢复后静默回源补齐。
- `/records` 页保留原有筛选、分页、排序、`snapshotId` 与 `newRecordsCount` 语义，同时只把“命中当前窗口”的 SSE 记录合并进当前页；窗口外增量继续通过 `New data` 提示暴露，不静默污染当前结果集。

## Verification

- `cd web && bun run test -- --run src/hooks/useInvocations.test.tsx src/hooks/useInvocationRecords.test.tsx src/pages/account-pool/UpstreamAccounts.test.tsx`
