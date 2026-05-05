# Codex 远程压缩请求记录、展示与计费接入 - Implementation

## Current State

- Canonical spec: `docs/specs/g3amk-codex-remote-compact-observability/SPEC.md`
- Implementation summary: 已完成（5/5）

## Migrated Implementation Notes

## 状态

- Status: 已完成（5/5）
- Created: 2026-03-09
- Last: 2026-04-27

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust tests：覆盖 compact 路径命中、payload endpoint、usage / cost 落库、summary / timeseries 包含 compact、compact 不触发 rewrite。
- Vitest：覆盖 InvocationTable 主列表 compact 标记与详情仍显示原始 endpoint。
- Playwright：继续校验 Dashboard / Live 响应式布局无新增 overflow，且 compact 标记在桌面 / 移动可见。

## 文档更新（Docs to Update）

- `docs/specs/README.md`：新增规格索引并同步状态。
