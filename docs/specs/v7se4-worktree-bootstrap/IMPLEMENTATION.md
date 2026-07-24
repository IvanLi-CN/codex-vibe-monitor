# Worktree bootstrap 与显式依赖初始化 实现状态（#v7se4）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 已实现
- Lifecycle: active
- Catalog note: linked `post-checkout` 自动恢复依赖，主 worktree 跳过；手动 bootstrap 保留失败码。

## Coverage / rollout summary

- `scripts/worktree-bootstrap.sh` 安装 shared hooks、同步缺失本地资源并调用依赖 setup。
- `scripts/worktree-setup.sh` 逐项执行三项 `bun install --frozen-lockfile` 与 `cargo fetch --locked`，汇总失败。
- `scripts/run-lefthook-hook.sh` 仅在 linked worktree 的 `post-checkout` 调用依赖 setup，并吞掉失败码。
- `scripts/test-worktree-bootstrap.sh` 使用真实 linked worktree smoke 和 fake `bun`/`cargo` 验证自动/手动入口、主 worktree 跳过与失败隔离。
- README 与 AGENTS 已说明自动恢复、手动失败码和 locked 参数。

## Remaining Gaps

- None

## Related Changes

- 扩展 `bun run worktree:setup` 覆盖 Rust 与 locked install。
- 将依赖恢复接入 linked `post-checkout` 和手动 `worktree:bootstrap`。
- 扩展 worktree bootstrap smoke test 的失败隔离与退出码覆盖。

## References

- `./SPEC.md`
- `./HISTORY.md`
