# API Key 优先级迁移准入控制 实现状态

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: backend, audit contract, and Settings UI implemented; final validation in progress
- Lifecycle: active
- Catalog note: topic anchor: API Key / routing / sticky priority handoff

## Coverage / rollout summary

- 当前路由仅在可复用的 `Fallback` sticky 来源上执行主动高优先级比较；成功后通过 generation-guarded sticky 写入提交。
- 当前模型路由健康已按 API Key 的精确请求模型维护冷却与恢复状态，但其持久化状态不是新的优先级迁移许可权威。
- 本主题完成后，全局 Settings 开关默认开启；关闭时保留当前旧路由行为，重新开启时以新的本地状态代际重新验证。
- 运行时许可、冷却与恢复计数只存在于当前进程。设置和诊断可持久化，但没有持久化可用性时不得阻断请求。

## Planned implementation map

- `src/upstream_accounts/routing/priority_handoff.rs` 提供进程本地、按账号与规范化请求模型隔离的 RAII 许可、代际、冷却和三次成功恢复状态机；准入代际写入 attempt 审计，旧代际终态不会污染重新开启后的验证；数据库查询仅用于从已有 attempt 记录补充结果，失败时许可仍由本地对象释放。
- `src/upstream_accounts/routing/selection.rs` 在既有候选排序和 `Fallback` sticky 边界之后执行准入；许可忙或冷却时 sticky 保留原来源，新分配继续扫描其他候选，不创建等待队列。
- `src/proxy/failover.rs` 为已准入的 API Key HTTP 尝试设置单次账号预算、禁用 429/过载重试和自动故障切换；终态记录路径驱动成功、临时失败和取消释放。
- `src/upstream_accounts/routing/settings_runtime.rs`、`core_schema_maintenance.rs` 与设置 API 增加默认开启的 `priorityHandoffAdmissionEnabled`，本地镜像优先于持久化诊断，旧数据库通过列迁移兼容。
- 路由审计增加可选 `handoffAdmission` 快照，使用安全阶段与恢复计数；`web/src/lib/api/` 保持旧 payload 的归一化结果稳定。
- `web/src/features/settings/PoolRoutingSettingsCard.tsx` 和 `Settings.tsx` 增加全局 HTTP/API Key 优先级迁移开关、保存状态和中英文文案，Storybook fixtures 已覆盖默认值。
- 已加入状态机、代际、数据库故障隔离、审计结构和现有 routing settings 的 Rust 回归覆盖；Web 类型检查、单测、构建和 Storybook 设置故事覆盖已通过，完整视觉证据与 PR 收敛待最终门禁执行。

## Remaining Gaps

- 真实上游端到端的请求取消/交付不确定性仍依赖现有传输 harness；本地终态路径已按保守规则处理，最终 PR 仍需完成视觉证据与 CI 收敛。

## Related Changes

- `e46acd37 docs(routing): define priority handoff admission contract`

## References

- `./SPEC.md`
- `./HISTORY.md`
- `../../adr/0002-stage-automatic-priority-handoffs.md`
