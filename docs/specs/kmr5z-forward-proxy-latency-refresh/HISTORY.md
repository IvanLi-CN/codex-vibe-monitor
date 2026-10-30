# Forward Proxy 节点延迟与订阅刷新 - 历史（#kmr5z）

> 这里记录会影响 Agent 理解“为什么一步步变成现在这样”的关键演进；单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 新增本 spec，用于固定 Settings 页 forward proxy 手动测速、批量广度优先调度、渐进均值显示与手动订阅刷新契约。
- 手动延迟测试从“任一探针成功即可显示正常”升级为“出站 IP、OAuth `/models`、Codex `/responses` 全部可达才健康”，用于暴露通用探针可过但 `chatgpt.com/backend-api/codex/responses` 不可达的节点异常。
- 订阅解析结果此前只保存在 `ForwardProxyManager` 内存中，重启后不能从持久化数据恢复；新增与订阅源配置绑定的快照，并在所有来源成功后原子替换。
- 刷新失败或部分失败保留上次成功节点；完整成功但解析为空时立即清空节点，以保持空订阅结果的明确语义。
- OAuth live-candidate 之前只按会话或分组 scope 准备最终请求，遗漏了账号级绑定；现改为复用既有 `conversation > account > group` 代理继承契约，并保留 API-key 的 transit scope。
- 刷新任务的互斥范围扩展到整个抓取与应用周期，并用受控慢源回归验证旧响应不能覆盖后续刷新结果；OAuth 回归同时覆盖过期 token 的账号级代理刷新。
- 快照恢复进一步收紧为“全量规范化且全部可解析”才接受，避免损坏 JSON 中的单个非法节点被静默丢弃后形成不完整恢复；部分刷新继续保留既有手动 HTTP 响应契约，并将不完整结果记录为内部 `skipped` 任务。

## Key Reasons / Replacements

- 手动测速属于长期可复用的 forward proxy 诊断能力，不应只作为一次性 UI 任务留在 PR 描述中。
- `/responses` 是线上 `/v1/responses` 转发的实际上游端点；只测试 `/models` 不能覆盖端点级网络阻断，因此延迟测试必须把 `/responses` 纳入目标集，并将 `405` 等无凭据 `<500` 响应视为可达性成功。
- 节点快照记录其订阅源配置，因此重新配置不会误用旧节点；旧版本仍可读写现有设置列，迁移触发器会在旧版本修改订阅源时清除快照。
- OAuth 代理优先级的长期契约仍由 [`r4p9x`](../r4p9x-upstream-account-policy-inheritance/SPEC.md) 维护，本次只修复 live-candidate 实现路径与该契约的偏差。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
