# 号池工作状态新增“不可用（不可调度）” - Implementation

## Current State

- Canonical spec: `docs/specs/m4k2q-upstream-account-unavailable-work-status/SPEC.md`
- Implementation summary: 已实现，待 PR / CI 收敛

## Migrated Implementation Notes

## 状态

- Status: 已实现，待 PR / CI 收敛
- Created: 2026-03-27
- Last: 2026-04-08

## Migrated Task-Ticket Sections

## 里程碑（Milestones）

- [x] M1: 新增 follow-up spec，冻结 `unavailable` 的调度语义与筛选契约。
- [x] M2: 后端读模型与列表 query 支持 `workStatus=unavailable`。
- [x] M3: 前端筛选、详情头部、翻译与 Storybook mock 统一到 `unavailable`。
- [x] M4: Storybook 覆盖 `unavailable` 三类异常并完成视觉证据。
- [ ] M5: 验证、review-loop 与 PR 收敛到 merge-ready。
