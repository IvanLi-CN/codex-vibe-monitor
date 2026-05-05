# InvocationTable 响应式修复：lg+ 无横向滚动、sm 及以下列表化 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/r8m3k-invocation-table-responsive-no-overflow/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-02: 创建规格，进入实现阶段。
- 2026-03-02: `InvocationTable` 完成双视图改造（`<768` 列表卡片、`>=768` 表格），并统一展开详情行为。
- 2026-03-02: 表格视图收敛列宽预算与长文本截断策略，覆盖长 endpoint / 长错误串场景，`lg+` 无横向滚动。
- 2026-03-02: E2E 扩展为 Dashboard/Live × `375/768/1024/1280/1440/1873` 全矩阵，校验视图形态、溢出约束与详情交互。
- 2026-03-02: 本地验证通过：`npm run test`、`npm run build`、`E2E_BASE_URL=http://127.0.0.1:4173 npm run test:e2e -- tests/e2e/invocation-table-layout.spec.ts`。
- 2026-03-02: 快车道交付 PR [#79](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/79)，标签 `type:patch` + `channel:stable`。
- 2026-03-02: 修复 `768px` 展开后右侧留白：按 `xl` 可见列动态设置 `colSpan`，并重配 `md/xl` 列宽预算；E2E 新增“首行右侧空隙 <= 1px”断言。
