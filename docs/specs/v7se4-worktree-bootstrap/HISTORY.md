# Worktree bootstrap 与显式依赖初始化 演进历史（#v7se4）

> 这里记录会影响 Agent 理解“为什么一步步变成现在这样”的关键演进；单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 2026-03-14: archived spec 固定 shared hooks、`post-checkout`、`.env.local` copy-missing-only 与真实 linked worktree smoke。
- 2026-05-15: 重新建立 canonical `docs/specs/` 主题 spec，并将依赖安装明确拆到显式 `worktree:setup`，避免 checkout hook 变成联网/重型动作。
- 2026-07-24: 将依赖恢复扩展到 linked `post-checkout`；三项 Bun 安装和 `cargo fetch --locked` 逐项执行，自动路径告警后继续 checkout，手动 bootstrap 返回聚合失败码。

## Key Reasons / Replacements

- linked worktree 的自动 bootstrap 需要同时恢复依赖，但主 worktree 的普通 checkout 不应承担这类网络动作。
- 依赖任务必须逐项隔离；自动 hook 不能阻断 Git checkout，手动入口则必须暴露失败。
- `worktree:setup` 继续作为共享的依赖恢复实现，`worktree:bootstrap` 负责在资源同步后调用它。
- 本 spec 继承并取代 archived `docs/archive/specs/v7se4-worktree-bootstrap/SPEC.md` 作为当前有效规范。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
- `docs/archive/specs/v7se4-worktree-bootstrap/SPEC.md`
