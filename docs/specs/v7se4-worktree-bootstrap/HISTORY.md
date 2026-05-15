# Worktree bootstrap 与显式依赖初始化 演进历史（#v7se4）

> 这里记录会影响 Agent 理解“为什么一步步变成现在这样”的关键演进；单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 2026-03-14: archived spec 固定 shared hooks、`post-checkout`、`.env.local` copy-missing-only 与真实 linked worktree smoke。
- 2026-05-15: 重新建立 canonical `docs/specs/` 主题 spec，并将依赖安装明确拆到显式 `worktree:setup`，避免 checkout hook 变成联网/重型动作。

## Key Reasons / Replacements

- 自动 bootstrap 适合补缺失本地配置，不适合安装依赖；依赖安装失败不应阻断普通 Git checkout。
- `worktree:setup` 是完整开发环境初始化入口，职责与 `worktree:bootstrap` 分离。
- 本 spec 继承并取代 archived `docs/archive/specs/v7se4-worktree-bootstrap/SPEC.md` 作为当前有效规范。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
- `docs/archive/specs/v7se4-worktree-bootstrap/SPEC.md`
