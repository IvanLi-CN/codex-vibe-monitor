# 优雅停机补强 - Implementation

## Current State

- Canonical spec: `docs/specs/fmfuf-graceful-shutdown-hardening/SPEC.md`
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-03-10
- Last: 2026-03-10

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust tests：覆盖运行时停机编排、HTTP graceful shutdown、Xray soft terminate / fallback kill、detached worker shutdown 短路。
- 回归测试需保持现有 proxy / forward proxy 行为不变，不新增对外接口断言变更。

## 文档更新（Docs to Update）

- `docs/specs/README.md`：新增 spec 索引，并在交付完成后同步状态、PR 与 checks 结果。
- `docs/specs/fmfuf-graceful-shutdown-hardening/SPEC.md`：记录验收、测试结果与最终交付状态。
