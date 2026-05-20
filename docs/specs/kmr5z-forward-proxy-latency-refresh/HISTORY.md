# Forward Proxy 节点延迟与订阅刷新 - 历史（#kmr5z）

> 这里记录会影响 Agent 理解“为什么一步步变成现在这样”的关键演进；单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 新增本 spec，用于固定 Settings 页 forward proxy 手动测速、批量广度优先调度、渐进均值显示与手动订阅刷新契约。
- 手动延迟测试从“任一探针成功即可显示正常”升级为“出站 IP、OAuth `/models`、Codex `/responses` 全部可达才健康”，用于暴露通用探针可过但 `chatgpt.com/backend-api/codex/responses` 不可达的节点异常。

## Key Reasons / Replacements

- 手动测速属于长期可复用的 forward proxy 诊断能力，不应只作为一次性 UI 任务留在 PR 描述中。
- `/responses` 是线上 `/v1/responses` 转发的实际上游端点；只测试 `/models` 不能覆盖端点级网络阻断，因此延迟测试必须把 `/responses` 纳入目标集，并将 `405` 等无凭据 `<500` 响应视为可达性成功。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
