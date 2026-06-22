# 系统工作区重构 - Implementation

## Current State

- Canonical spec: `docs/specs/s7m3q-system-workspace/SPEC.md`
- Status: 实现中

## Implementation Summary

- 新增 `system` 顶层工作区与四个子页：`状态 / 任务 / 设置 / 代理`。
- 顶层导航由 `设置` 改为 `系统`，旧 `#/settings` 改为兼容跳转。
- 新增系统状态接口与系统后台任务记录接口。
- 原 settings 页按职责拆分为通用设置页与 forward-proxy 页，同时继续复用现有设置数据模型与写接口。
- 系统状态页 raw 统计已切换为真实磁盘文件口径，并拆分为 `raw / request / response` 三组指标。

## Quality Gates

- `cargo test`
- `cd web && bun run test`
- `cd web && bun run build`
- `cd web && bun run build-storybook`

## Disposition

- `spec_disposition=create`
- `project_doc_disposition=none`
- `solution_disposition=none`
