# 上游账号列表分组筛选 实现状态（#mmdms）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: implemented
- Lifecycle: active
- Catalog note: account pool roster filtering

## Coverage / rollout summary

- 账号列表分组筛选从单选/自由文本迁移为精确多选，触发器展示选中摘要。
- 后端 `groupExact` 支持重复查询参数，并以 OR 语义匹配账号分组。
- 空白账号分组由 schema 维护与写入路径归一化为 `未分组`。
- 分组候选基于全量 `groups[].accountCount > 0`，不受工作状态、启用状态、账号状态、标签或当前分组筛选影响。
- 分组候选右侧展示 `xN` 账号数量；仅当前已选但当前无账号的保留项展示 `x0`。
- 旧 `groupFilter` 本地存储读取时迁移为 `groupFilters` 数组。

## Remaining Gaps

- None

## Related Changes

- Backend: repeated `groupExact` parsing, list OR filtering, blank group schema maintenance, and write-path normalization.
- Web: persisted filter migration, multi-select group combobox, repeated query serialization, Storybook list interaction coverage.
- Evidence: `./assets/group-filter-multiselect-catalog.png`

## References

- `./SPEC.md`
- `./HISTORY.md`
