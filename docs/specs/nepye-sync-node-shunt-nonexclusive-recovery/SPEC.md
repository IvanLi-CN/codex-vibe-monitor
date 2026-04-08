# node shunt 同步路径共享绑定节点恢复（#nepye）

## 状态

- Status: 已实现，待 PR / CI / review-proof 收敛
- Created: 2026-04-07
- Last: 2026-04-07

## Summary

- 修复启用 `nodeShuntEnabled` 的 OAuth 分组在同步路径误复用“独占节点槽位”分配的问题：手动同步、批量同步与 maintenance sync 不再因为 `分组节点分流策略控制，未排节点` 被假阻断。
- 调用期独占路由保持不变；只放宽同步探测。fresh assignment / sticky reuse 仍必须命中独占槽位，列表与详情里的 `group_node_shunt_unassigned` 读模型语义继续保留。
- 同步优先复用当前账号已有 live reservation / 当前排到的固定节点；若当前账号没有独占槽位，则回退到该分组的 shared bound-group probe，对绑定节点做非独占 usage probe。
- 若同步结果仍是 `429 quota exhausted` 或 `sync_recovery_blocked`，账号继续保持既有 exhausted / recovery gate 终态，不回流为独占槽位持有者，也不创建新的全局占用。

## Scope

### In scope

- `src/upstream_accounts/mod.rs`
  - OAuth sync-only forward proxy resolver
  - manual sync / bulk sync / maintenance sync 统一选路
  - node shunt sync fallback 与回归测试
- `docs/specs/README.md`
- `docs/specs/g4ek6-account-pool-upstream-accounts/contracts/http-apis.md`

### Out of scope

- fresh assignment / sticky reuse 的调用期独占槽位分配规则
- 前端页面、文案与视觉证据
- 非 OAuth 账号的同步路由改造
- 额度窗口计算、429 分类、恢复 gate 本身的既有语义

## Requirements

### MUST

- 当账号分组启用 `nodeShuntEnabled` 时，OAuth 同步路径必须使用独立的 sync-only resolver，而不是直接复用调用期 `resolve_account_forward_proxy_scope()` 的独占槽位检查。
- sync-only resolver 必须按以下优先级选路：
  1. 当前账号已有的 live pinned reservation；
  2. 当前账号在 node shunt 分配器中的既有固定节点；
  3. 同组 selectable `boundProxyKeys` 组成的 shared bound-group probe。
- sync-only resolver 仍必须保留真实 misconfiguration 阻断：
  - 缺少 group name
  - 分组未绑定节点
  - 绑定节点全都不可选 / 不可用
- manual sync、bulk sync job、maintenance sync 必须共用同一条 sync-only resolver 语义，不允许继续存在“某条同步入口放宽、另一条仍报未排节点”的分叉。
- `group_node_shunt_unassigned` 仍只代表调用期没有独占节点槽位；列表与详情导出的 `routingBlockReason*` 不得因为本次修复被移除或改写。
- 同步命中 `429 quota exhausted`、`quota_still_exhausted` 或其他既有 recovery-blocked 终态后，不得创建/保留新的独占 reservation，也不得把该账号重新纳入 node shunt eligible 槽位。

### SHOULD

- 当当前账号已经持有 live reservation 时，同步应继续复用该节点，避免一次同步内先后打到不同节点。
- shared bound-group probe 只应在 sync-only 场景触发，不应泄漏到 pool live request 或其他调用期独占语义。

## Acceptance

- Given 分组启用 `nodeShuntEnabled` 且只绑定 1 个 selectable 节点，When 该节点已被 working 账号占用且另一账号处于 `quota_exhausted / sync_recovery_blocked`，Then 手动同步、批量同步与 maintenance sync 都必须对该分组 bound node 发起真实 usage probe，而不是直接报 `分组节点分流策略控制，未排节点`。
- Given 当前账号已经持有 live reservation，When 对该账号执行同步，Then sync-only resolver 必须继续复用该 pinned node，而不是退回 shared bound-group probe。
- Given 当前账号在调用期仍未排到独占槽位，When 读取列表或详情，Then `routingBlockReasonCode=group_node_shunt_unassigned` 与 `routingBlockReasonMessage=分组节点分流策略控制，未排节点` 仍然保留。
- Given 同步结果仍为 `429 quota exhausted`，When 同步结束，Then 账号继续保持 exhausted / recovery-blocked 终态，且 node shunt 分配器不会因此把它重新视为占槽账号。
- Given API key 账号或 pool live request 的调用期路由，When 分组启用 `nodeShuntEnabled`，Then 仍必须沿用独占槽位分配语义，不得因本次修复退化成 shared bound-group probe。

## Validation

- `cargo test sync_scope_reuses_live_reserved_node_for_same_account_before_shared_group_probe -- --test-threads=1`
- `cargo test sync_scope_falls_back_to_shared_bound_group_when_exclusive_slot_is_full -- --test-threads=1`
- `cargo test manual_sync_allows_group_node_shunt_unassigned_account_to_probe_bound_node -- --test-threads=1`
- `cargo test maintenance_sync_allows_group_node_shunt_unassigned_account_to_probe_bound_node -- --test-threads=1`
- `cargo test bulk_sync_allows_group_node_shunt_unassigned_account_to_probe_bound_node -- --test-threads=1`
- `cargo test detail_preserves_group_node_shunt_unassigned_routing_block_reason -- --test-threads=1`
- `cargo fmt --check`
- `cargo check`
- `cargo test`

## Risks / Notes

- 本次修复是 `#6b9ra` 的 follow-up：原始 spec 对“所有账号上下文请求都必须命中独占节点”作了统一收口，但线上恢复场景证明同步探测与真实调用期路由需要拆分语义。
- 只有 sync-only resolver 放宽；如果后续有新的“账号上下文 maintenance”入口，必须显式判断它属于同步探测还是调用期流量。
- 本次不改 HTTP/TS shape；前端仍通过既有 `routingBlockReason*` 与同步结果渲染状态。

## Change log

- 2026-04-07: 创建 follow-up spec，冻结“仅放宽 OAuth sync path、调用期独占语义不变”的修复边界与验收口径。
- 2026-04-07: 完成后端 sync-only resolver、manual/bulk/maintenance sync 回归测试，以及 `cargo fmt --check`、`cargo check`、`cargo test` 本地验证。
