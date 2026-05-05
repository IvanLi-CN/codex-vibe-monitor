# 请求实况即时展示与“用时”订正 - Implementation

## Current State

- Canonical spec: `docs/specs/mj5nt-live-running-elapsed-sse/SPEC.md`
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-03-17
- Last: 2026-03-17

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: Rust 覆盖临时 SSE 快照与终态替换；Vitest 覆盖 `useInvocationStream`/`InvocationTable` 合并与“用时”展示。
- Integration tests: 代理请求生命周期中 `records` SSE 的 running -> enriched running -> terminal 顺序。
- E2E tests (if applicable): Dashboard / Live 共享 InvocationTable 的即时展示回归。

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 登记本规格并同步最终状态
