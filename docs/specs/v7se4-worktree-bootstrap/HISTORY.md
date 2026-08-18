# Worktree bootstrap 与显式依赖初始化 演进历史（#v7se4）

> 这里记录会影响 Agent 理解“为什么一步步变成现在这样”的关键演进；单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 2026-03-14: archived spec 固定 shared hooks、`post-checkout`、`.env.local` copy-missing-only 与真实 linked worktree smoke。
- 2026-05-15: 重新建立 canonical `docs/specs/` 主题 spec，并将依赖安装明确拆到显式 `worktree:setup`，避免 checkout hook 变成联网/重型动作。
- 2026-07-24: 将依赖恢复扩展到 linked `post-checkout`；三项 Bun 安装和 `cargo fetch --locked` 逐项执行，自动路径告警后继续 checkout，手动 bootstrap 返回聚合失败码。
- 2026-08-14: 将 hook 入口收敛为 Lefthook-only；标准 `post-checkout` hook 调用 runner，smoke 改为真实 Lefthook 触发并验证 Vitest sentinel、历史 revision no-op 与全局 Lefthook 前置条件。
- 2026-08-18: checkout setup 改为 per-worktree、per-surface digest 状态；普通切换跳过 Bun/Cargo，failed digest 自动抑制，手动入口重试或 force，资源锁改为非阻塞且隔离。
- 2026-08-18: 将资源与 setup 锁收敛为持锁进程生命周期内的 advisory lock，避免 stale PID 删除竞态；formatter、hook 安装和历史 checkout 分别收紧 symlink、模板归属与 no-op 边界。
- 2026-08-18: 将 formatter containment 扩展到所有路径组件；模板比较改为带目标 hook 配置的临时仓库；Cargo `ok` 状态加入 registry archive presence，并以真实手动串行、Cargo 失败和临时 repo 外 Lefthook smoke 固化边界。
- 2026-08-18: 将 dependency readiness 从单一 Bun/Cargo 哨兵收敛为 manifest 直接 package 与 `Cargo.lock` registry archive 集合；CI tooling job 安装全局 Lefthook，smoke 不再复制 repo-local launcher。
- 2026-08-19: Cargo readiness 状态保留已验证 archive 的 cache 相对布局，防止同名 archive 出现在另一 registry cache 时掩盖原位置缺失；partial-stage smoke 以 formatter 重写验证 formatter wrapper 在部分暂存时停止、完整暂存时只提交格式化结果，同时覆盖 linked worktree 中 stale shared Lefthook patch 不得绕过直接工作树检查及已标记旧 standard pre-commit 迁移。

## Key Reasons / Replacements

- linked worktree 的自动 bootstrap 需要恢复依赖，但主和 linked worktree 的普通 checkout 都不应重复承担网络或编译动作。
- 依赖任务必须逐项隔离；自动 hook 不能阻断 Git checkout，手动入口则必须暴露失败。
- `worktree:setup` 继续作为共享的依赖恢复实现，`worktree:bootstrap` 负责在资源同步后调用它。
- 依赖状态必须位于每个 worktree 的 Git metadata，避免共享锁、secret 和失败重试在 worktree 间泄漏。
- 本 spec 继承并取代 archived `docs/archive/specs/v7se4-worktree-bootstrap/SPEC.md` 作为当前有效规范。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
- `docs/archive/specs/v7se4-worktree-bootstrap/SPEC.md`
