# MITM 请求级计费与性能采集（Proxy as Source of Truth） - Implementation

## Current State

- Canonical spec: `docs/specs/0007-proxy-mitm-usage-billing/SPEC.md`
- Migrated from legacy source: `docs/plan/0007:proxy-mitm-usage-billing/PLAN.md`
- Legacy source retention: pending delete approval
- Implementation summary: See companion notes and linked PR/check history for implementation context.

## Migrated Implementation Notes

## 测试策略

- Rust 单元测试：usage 提取、流式 include_usage 注入、成本估算、阶段耗时聚合。
- Rust 集成测试：`chat`/`responses` 代理采集链路、流式中断与降级行为。
- 回归验证：`cargo test`、`cargo check`、前端最小类型检查（如触及 web）。

## 里程碑

- [x] M1 代理采集框架与 schema 扩展（含阶段耗时字段）
- [x] M2 chat/responses usage 解析 + 成本估算 + 原文落盘
- [x] M3 聚合接口与性能统计接口
- [x] M4 验证、文档与回归测试
