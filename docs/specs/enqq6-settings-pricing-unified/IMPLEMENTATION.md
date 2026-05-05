# 统一设置页：代理配置 + 价格配置（替换旧方案） - Implementation

## Current State

- Canonical spec: `docs/specs/enqq6-settings-pricing-unified/SPEC.md`
- Migrated from legacy source: `docs/plan/enqq6-settings-pricing-unified/PLAN.md`
- Legacy source retention: pending delete approval
- Implementation summary: See companion notes and linked PR/check history for implementation context.

## Migrated Implementation Notes

## Testing

- Backend: `cargo test`（覆盖 settings API、持久化、代理模型行为与价格热更新）。
- Backend: `cargo check`。
- Frontend: `cd web && npm run test`。
- Frontend: `cd web && npm run build`。
- E2E: 更新并执行设置相关 Playwright 用例。

## Milestones

- [x] M1 requirements freeze 与 docs/plan 索引更新
- [x] M2 后端新 settings API + pricing DB 持久化
- [x] M3 前端 `/settings` 页面与自动保存
- [x] M4 测试通过、PR 创建并完成 checks 跟踪
