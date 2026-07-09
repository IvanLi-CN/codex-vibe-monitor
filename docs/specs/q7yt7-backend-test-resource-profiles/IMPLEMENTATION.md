# 后端测试资源分层模块化与运行时预算 实现状态（#q7yt7）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 部分完成（PR1 测试树模块化已落地，PR2 runtime/CI 合同待完成）
- Lifecycle: active
- Catalog note: 双 PR：先测试树模块化，再 runtime/CI 合同收口

## Coverage / rollout summary

- 当前计划分为两个连续 PR：
  - PR1：深层测试模块化
  - PR2：profile-aware runner、required-check 拆分与 runtime budget 收口
- PR1 已将 `src/tests/slices` 与 `src/upstream_accounts/tests_part_*` 迁入真实模块树：
  - `src/tests/{lightweight,stateful_sqlite,archive_file_io}`
  - `src/upstream_accounts/tests/{lightweight,stateful_sqlite,archive_file_io}`
- PR1 已移除旧字母/编号切片文件名与 `src/upstream_accounts/tests/parts.rs` 聚合入口。
- 当前模块树仍通过最小必要的 `pub(crate)` helper 暴露跨文件测试支撑；PR2 不再扩展这类聚合面，只围绕 runner/CI/runtime 收口。

## Remaining Gaps

- 待补充：三个 backend profiles 的 runner 入口与 nextest 过滤真相源。
- 待补充：quality-gates / release snapshot / release gate 的 required-check 合同迁移完成度。
- 待补充：split 后各 profile wall time 与 top offenders 证据。

## Related Changes

- PR1 branch: `th/backend-test-modularization`

## References

- `./SPEC.md`
- `./HISTORY.md`
