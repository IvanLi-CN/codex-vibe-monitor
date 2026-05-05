# node shunt 同步路径共享绑定节点恢复 - Implementation

## Current State

- Canonical spec: `docs/specs/nepye-sync-node-shunt-nonexclusive-recovery/SPEC.md`
- Implementation summary: 已实现，待 PR / CI / review-proof 收敛

## Migrated Implementation Notes

## 状态

- Status: 已实现，待 PR / CI / review-proof 收敛
- Created: 2026-04-07
- Last: 2026-04-07

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
