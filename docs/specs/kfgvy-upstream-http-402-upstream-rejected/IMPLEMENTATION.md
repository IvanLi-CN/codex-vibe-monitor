# 402 `deactivated_workspace` 账号状态改判为上游拒绝 - Implementation

## Current State

- Canonical spec: `docs/specs/kfgvy-upstream-http-402-upstream-rejected/SPEC.md`
- Implementation summary: 已完成（5/5，PR #244）

## Migrated Implementation Notes

## 状态

- Status: 已完成（5/5，PR #244）
- Created: 2026-03-26
- Last: 2026-04-08

## Migrated Task-Ticket Sections

## 里程碑（Milestones）

- [x] M1: 创建 follow-up spec 并冻结 `402 -> upstream_rejected` 的导出语义。
- [x] M2: 后端状态派生改为优先消费结构化 `402` 信号，并补齐 route/sync 双回归。
- [x] M3: 补齐 Storybook 402 场景与前端断言，固定列表/详情显示结果。
- [x] M4: 完成本地验证、Storybook 视觉证据与浏览器 smoke。
- [x] M5: 快车道收敛到 merge-ready PR，并回填 spec / README 状态。
