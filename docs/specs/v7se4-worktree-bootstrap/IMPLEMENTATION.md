# Worktree bootstrap 与显式依赖初始化 实现状态（#v7se4）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 已实现
- Lifecycle: active
- Catalog note: `post-checkout` bootstrap 保持轻量；依赖安装由 `worktree:setup` 显式触发。

## Coverage / rollout summary

- `scripts/worktree-bootstrap.sh` 继续只安装 shared hooks 并同步缺失本地资源。
- `scripts/worktree-setup.sh` 安装 repo root、`web/`、`docs-site/` Bun 依赖。
- `scripts/test-worktree-bootstrap.sh` 使用真实 linked worktree smoke 和 fake `bun` 验证 no-deps bootstrap 与 setup 调用链。
- README 与 AGENTS 已说明 bootstrap/setup 的职责差异。

## Remaining Gaps

- None

## Related Changes

- 新增 `bun run worktree:setup`。
- 扩展 worktree bootstrap smoke test。

## References

- `./SPEC.md`
- `./HISTORY.md`
