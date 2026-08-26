# API Key 优先级迁移准入控制 实现状态

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 未开始
- Lifecycle: active
- Catalog note: topic anchor: API Key / routing / sticky priority handoff

## Coverage / rollout summary

- 当前路由仅在可复用的 `Fallback` sticky 来源上执行主动高优先级比较；成功后通过 generation-guarded sticky 写入提交。
- 当前模型路由健康已按 API Key 的精确请求模型维护冷却与恢复状态，但其持久化状态不是新的优先级迁移许可权威。
- 本主题完成后，全局 Settings 开关默认开启；关闭时保留当前旧路由行为，重新开启时以新的本地状态代际重新验证。
- 运行时许可、冷却与恢复计数只存在于当前进程。设置和诊断可持久化，但没有持久化可用性时不得阻断请求。

## Planned implementation map

- 在 `src/app_state.rs` 增加小型、取消安全的优先级迁移运行时状态：按目标账号与规范化请求模型保存许可、阶段、冷却证据、恢复计数与全局设置代际。
- 在 `src/upstream_accounts/routing/selection.rs` 保留现有比较器和 `Fallback` 边界，在候选已选定后附加迁移准入决定、延期原因和新分配绕行信息；不得把许可冲突映射为常规候选排序变化。
- 在 `src/proxy/{route_selection.rs,failover.rs}` 标记闸门准入的 HTTP 尝试，强制单次发送和终态提交；复用现有 sticky generation guard，并实现确定未送达时的单次安全回放。
- 在 `src/upstream_accounts/routing/{model_health.rs,failure_recording.rs}` 与本地运行时之间接入临时失败、冷却、成功和人工 reset 信号；本地转换先完成，持久化模型健康仅作兼容性与诊断同步。
- 在 `src/{schema.rs,app_state.rs}` 及既有 routing settings API 中增加 `priorityHandoffAdmissionEnabled` 的全局存储、读取、写入和本地镜像更新，保持 Settings 写入的原子语义。
- 在 `src/proxy/usage_persistence.rs`、路由审计投影和模型路由事件路径中补充安全的迁移阶段/结果字段；记录失败不能回写影响运行时许可。
- 在 `web/src/features/settings/`、`web/src/lib/api/`、现有 Records 与路由实况组件中增加全局开关和审计文案，不新增独立控制面。
- 在对应 Rust stateful SQLite 测试模块、Settings/Records Vitest 与 Storybook 中覆盖并发和 UI 状态。

## Remaining Gaps

- 运行时状态机、HTTP 发送策略、Settings 字段、审计字段和 UI 尚未实现。
- 尚未运行本主题的 Rust、Web 或视觉验证。

## Related Changes

- None

## References

- `./SPEC.md`
- `./HISTORY.md`
- `../../adr/0002-stage-automatic-priority-handoffs.md`
