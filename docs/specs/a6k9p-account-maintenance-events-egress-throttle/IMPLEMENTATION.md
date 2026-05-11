# 账号池维护执行记录与出口限频 - Implementation

## Current State

- Canonical spec: `docs/specs/a6k9p-account-maintenance-events-egress-throttle/SPEC.md`
- Status: 已实现

## Implementation Summary

- 扩展 `pool_upstream_account_events`，补齐账号快照、分组、forward proxy、出口 IP、结果与结果描述字段。
- 新增 `pool_upstream_account_egress_throttle`，按最终出口记录最近真实外呼时间。
- 新增全局账号维护事件分页 API，并支持账号、分组、节点、结果筛选。
- 账号池新增 `维护记录` 独立页，承载“非模型调用执行记录”列表，包含执行时间列、四项筛选、分页和两行记录布局。
- 扩展 `forward_proxy_metadata_history`，通过 ipify 每 600 秒刷新一次 proxy/direct 出口 IP，维护事件写入时快照该 IP。
- 维护外呼在真实请求前预留 10 秒出口槽位；运行期维护同步遇到同出口槽位未释放时会在有界预算内等待并重试，预算耗尽后写入 deferred 事件，且账号不保持 `syncing` 状态。
- OAuth quota exhausted 账号不会按 reset time 自动退出限流；reset due 只触发后续 usage snapshot 维护同步，成功 snapshot 再按既有状态机保持或清除限流标记。
- `sync_deferred / egress_throttled` 不会消耗 reset catch-up 窗口；reset due 只会被真实同步尝试清掉，而普通维护间隔仍会把 deferred 记录当作最近一次尝试。

## Quality Gates

- `cargo fmt --check`
- `cargo check`
- `cargo test account_`
- `cargo test quota_exhausted -- --test-threads=1`
- `cargo test maintenance_reset_due -- --test-threads=1`
- `cargo test runtime_wait_retries_until_egress_slot_is_available -- --test-threads=1`
- `cargo test`
- `cd web && bun run test`
- `cd web && bun run build`
- `cd web && bun run test-storybook`
- Storybook 视觉证据

## Review Disposition

- Earlier review noted that OAuth refresh followed by usage sync can hit the same egress slot. Runtime maintenance now queues within a bounded wait budget, so reset-due OAuth accounts are not starved by immediate `sync_deferred / egress_throttled`; budget exhaustion still preserves the deferred path.
- `sync_deferred / egress_throttled` now preserves the post-reset catch-up window instead of consuming it, which keeps the next maintenance pass eligible to retry the real usage snapshot.

## Disposition

- `spec_disposition=update`
- `project_doc_disposition=none`
- `solution_disposition=none`
