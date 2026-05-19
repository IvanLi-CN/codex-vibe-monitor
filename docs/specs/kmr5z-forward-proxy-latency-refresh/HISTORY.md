# Forward Proxy 节点延迟与订阅刷新 - 历史（#kmr5z）

> 这里记录会影响 Agent 理解“为什么一步步变成现在这样”的关键演进；单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 新增本 spec，用于固定 Settings 页 forward proxy 手动测速、批量广度优先调度、渐进均值显示与手动订阅刷新契约。

## Key Reasons / Replacements

- 手动测速属于长期可复用的 forward proxy 诊断能力，不应只作为一次性 UI 任务留在 PR 描述中。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
