# 号池 `/v1/responses*` 超时护栏收口为 `180s / 300s` - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/t9m3p-pool-responses-timeout-guardrails/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-25: 修复 timeout route failover 与 exhausted 候选并存时的终态误分类；被 `route_key` 排除的健康候选现在会正确收敛到 `pool_no_alternate_upstream_after_timeout`，不再错误返回 `pool_all_accounts_rate_limited`。

- 2026-03-23: 创建 spec，冻结 `180s / 300s / 504` 的 timeout guardrails 边界、验收与验证要求。
- 2026-03-23: 完成本地实现与 targeted regression；待 fast-track 交付收口。
- 2026-03-23: 补齐总预算从首次 upstream 尝试起算、same-account retry 的 distinct-account 统计保持稳定，以及 OAuth `/v1/responses/compact` send-phase 也受总预算裁剪；本地 `cargo test` 全量通过。
