# Proxy 模型列表劫持与上游实时合并 - Implementation

## Current State

- Canonical spec: `docs/specs/0006-proxy-model-list-hijack/SPEC.md`
- Migrated from legacy source: `docs/plan/0006:proxy-model-list-hijack/PLAN.md`
- Legacy source retention: pending delete approval
- Implementation summary: See companion notes and linked PR/check history for implementation context.

## Migrated Implementation Notes

## 测试策略

- Rust：
  - 设置 API 读写与持久化测试。
  - `/v1/models` 三种行为分支测试。
  - 合并失败降级测试。
- Web：
  - 设置入口展示与交互测试（e2e）。
  - 开关保存失败回滚验证（可通过 mock 或测试环境 API 失败注入）。

## 里程碑

- [x] M1 数据模型与后端设置 API（含持久化）
- [x] M2 `/v1/models` 劫持与实时合并逻辑
- [x] M3 前端设置界面与自动化验证
