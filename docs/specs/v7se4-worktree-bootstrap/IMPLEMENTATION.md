# Worktree bootstrap 与显式依赖初始化 实现状态（#v7se4）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 已实现
- Lifecycle: active
- Catalog note: linked `post-checkout` 首次或失效时按 surface 恢复依赖，普通 checkout 跳过；手动 bootstrap 保留失败码。

## Coverage / rollout summary

- `bun run hooks:install` 通过 Lefthook 安装标准 shared hooks；`scripts/worktree-bootstrap.sh` 复用该入口、同步缺失本地资源并调用依赖 setup。
- `scripts/install-lefthook-hooks.sh` 拒绝 repo-local Lefthook 伪装全局前置条件，并以带目标 hook 配置的临时 Git 仓库生成模板；仅更新逐字匹配当前模板和 marker 的 managed hook，保留 `core.hooksPath` 与任意非精确匹配的本地 hook。
- `scripts/worktree-setup.sh` 为 root Bun、web Bun、docs Bun、Cargo 保存 per-worktree digest 状态；仅恢复首次、缺失或失效 surface，Bun `ok` 状态核验 manifest 的直接依赖 package，Cargo `ok` 状态核验 `Cargo.lock` 的 registry archive 集合及其 cache 相对布局指纹，failed digest 自动抑制，自动入口遇锁跳过，手动入口串行重试，`--force` 全量执行。
- `scripts/sync-worktree-resources.sh` 使用 per-worktree Git metadata 的非阻塞 advisory lock；持锁进程退出后锁自动释放，同步只复制缺失资源。
- `lefthook.yml` 的 `post-checkout` command 在 runner 缺失时安全 no-op；runner 仅在 linked worktree 调用自动 setup。installer 仅删除严格匹配当前 Lefthook 标准模板的遗留 `prepare-commit-msg` 包装器。
- `format-staged-files.sh --check` 以与三个 formatter 相同的路径过滤从 index 读取已暂存文件，直接检查工作树的同一目标 hunk。installer 只对精确匹配受管模板的 pre-commit 加入该检查，然后 Lefthook 继续以 `stage_fixed` 暂存 formatter 结果。
- CI tooling job 安装锁定的全局 Lefthook，两份 smoke 只接受 repo 外的 `PATH` 可执行文件；`scripts/test-worktree-bootstrap.sh` 使用真实 linked worktree 与 fake `bun`/`cargo` 验证主 worktree no-op、首次/重复/选择性恢复、单个 Bun package 与 Cargo archive 缺失恢复、失败抑制、手动重试、force、隔离 advisory 锁、手动串行与 copy-missing-only。
- README 与 AGENTS 已说明按指纹自动恢复、手动失败码、force 和 locked 参数。

## Remaining Gaps

- 无已知实现缺口；Lefthook PATH 前置条件、状态无敏感字段、Bun/Cargo dependency-set presence 与按指纹恢复已由维护文档与 smoke 覆盖。

## Related Changes

- 扩展 `bun run worktree:setup` 覆盖 Rust 与 locked install。
- 将依赖恢复接入 linked `post-checkout` 和手动 `worktree:bootstrap`。
- 扩展 worktree bootstrap smoke test 的选择性恢复、失败抑制、手动重试、force 与 per-worktree 锁覆盖。

## References

- `./SPEC.md`
- `./HISTORY.md`
