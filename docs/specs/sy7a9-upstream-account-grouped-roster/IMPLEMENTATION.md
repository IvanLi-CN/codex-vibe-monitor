# 上游账号列表分组视图与代理徽章 - Implementation

## Current State

- Canonical spec: `docs/specs/sy7a9-upstream-account-grouped-roster/SPEC.md`
- Implementation summary: See companion notes and linked PR/check history for implementation context.

## Verification

- See the canonical spec and linked PR/check history for verification details.

## Remaining Gaps

- None recorded in this migration.

## Migrated Task-Ticket Sections

## 里程碑（Milestones）

- [x] M1: 新建增量 spec，冻结视图切换、代理 badge 与 grouped roster 契约。
- [x] M2: 后端补齐 `includeAll`、当前代理读模型与 roster `forwardProxyNodes` catalog。
- [x] M3: 前端落地平铺/分组切换、分组卡片、分组设置入口与页面级组卡虚拟化。
- [x] M4: 补齐 Storybook 场景、Vitest/Rust 回归与视觉证据。
- [ ] M5: 快车道收敛到 merge-ready。
