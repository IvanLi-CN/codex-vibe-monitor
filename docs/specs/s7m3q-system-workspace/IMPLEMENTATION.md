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
- 系统状态页布局已从 12 张等权卡片重构为“项目磁盘总览 + 数据库记录概况 + 归档与逻辑体量”。
- 系统状态接口补充 `liveInvocationsCount` 与 `completedArchiveBatchesCount`，用于解释 live 数据库与归档来源。
- 系统状态页已把 `raw payload` 解释前置到数字旁：主读数旁展示项目总量公式，`raw payload` 总量显式标成“并集总量”，request / response 显式标成“侧向拆分”。
- `raw payload 聚焦` 已改成“总量卡 + request 行 + response 行”的纵向层级，去掉窄列中的并排四小卡，避免 request-heavy 场景下数字区被长说明挤压变形。
- 总览首屏已进一步改成顺序流：主读数、项目级 breakdown、`raw payload 聚焦` 依次堆叠，避免左右上半区高度失衡导致的巨大空白。

## Quality Gates

- `cargo test`
- `cd web && bun run test`
- `cd web && bun run build`
- `cd web && bun run build-storybook`

## Disposition

- `spec_disposition=create`
- `project_doc_disposition=none`
- `solution_disposition=none`
