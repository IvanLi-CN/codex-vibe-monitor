# 账号池维护执行记录与出口限频 - Implementation

## Current State

- Canonical spec: `docs/specs/a6k9p-account-maintenance-events-egress-throttle/SPEC.md`
- Status: 已实现

## Implementation Summary

- 扩展 `pool_upstream_account_events`，补齐账号快照、分组、forward proxy、出口 IP、结果与结果描述字段。
- 新增 `pool_upstream_account_egress_throttle`，按最终出口记录最近真实外呼时间。
- 新增全局账号维护事件分页 API，并支持账号、分组、节点、结果筛选。
- 账号池上游账号页新增“非模型调用执行记录”列表，包含执行时间列、四项筛选、分页和两行记录布局。
- 维护外呼在真实请求前预留 10 秒出口槽位；被限频时写入 deferred 事件，且账号不保持 `syncing` 状态。

## Quality Gates

- `cargo fmt --check`
- `cargo check`
- `cargo test account_`
- `cargo test`
- `cd web && bun run test`
- `cd web && bun run build`
- `cd web && bun run test-storybook`
- Storybook 视觉证据

## Review Disposition

- `codex review` raised that OAuth refresh followed by usage sync can defer the second request on the same egress. This is by design for this spec because both token refresh and usage snapshot are real non-model maintenance outbound calls, and the locked acceptance rule requires at least 10 seconds between any two real outbound calls on the same egress.

## Disposition

- `spec_disposition=create`
- `project_doc_disposition=none`
- `solution_disposition=none`
