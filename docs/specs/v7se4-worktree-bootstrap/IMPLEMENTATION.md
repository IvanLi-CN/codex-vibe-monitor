# Worktree bootstrap 与显式依赖初始化 实现状态（#v7se4）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 已实现
- Lifecycle: active
- Catalog note: linked `post-checkout` 首次或失效时按 surface 恢复依赖，普通 checkout 跳过；手动 bootstrap 保留失败码。

## Coverage / rollout summary

- `bun run hooks:install` 通过 Lefthook 安装标准 shared hooks；`scripts/worktree-bootstrap.sh` 复用该入口、同步缺失本地资源并调用依赖 setup。
- `scripts/install-lefthook-hooks.sh` 拒绝 repo-local Lefthook 伪装全局前置条件，并保留 `core.hooksPath` 与 unmanaged hook 的本地状态。
- `scripts/worktree-setup.sh` 为 root Bun、web Bun、docs Bun、Cargo 保存 per-worktree digest 状态；仅恢复首次、缺失或失效 surface，failed digest 自动抑制，手动入口重试，`--force` 全量执行。
- `scripts/sync-worktree-resources.sh` 使用 per-worktree Git metadata 的非阻塞锁；同步只复制缺失资源，busy 跳过且 stale lock 可恢复。
- `lefthook.yml` 的 `post-checkout` command 调用 `scripts/run-lefthook-hook.sh`；runner 仅在 linked worktree 调用自动 setup。installer 仅删除严格匹配当前 Lefthook 标准模板的遗留 `prepare-commit-msg` 包装器。
- `scripts/test-worktree-bootstrap.sh` 使用真实 Lefthook、真实 linked worktree smoke、fake `bun`/`cargo` 验证首次/重复/选择性恢复、失败抑制、手动重试、force、隔离锁与 copy-missing-only。
- README 与 AGENTS 已说明按指纹自动恢复、手动失败码、force 和 locked 参数。

## Remaining Gaps

- 无已知实现缺口；Lefthook PATH 前置条件、状态无敏感字段和按指纹恢复已由维护文档与 smoke 覆盖。

## Related Changes

- 扩展 `bun run worktree:setup` 覆盖 Rust 与 locked install。
- 将依赖恢复接入 linked `post-checkout` 和手动 `worktree:bootstrap`。
- 扩展 worktree bootstrap smoke test 的选择性恢复、失败抑制、手动重试、force 与 per-worktree 锁覆盖。

## References

- `./SPEC.md`
- `./HISTORY.md`
