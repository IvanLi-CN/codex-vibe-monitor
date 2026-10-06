# Dashboard 用量明细自适应数值显示主题历史

> 这里记录主题局部生命周期、兼容性与必要背景；规范正文以 `./SPEC.md` 为准。

## Lifecycle / Compatibility

- Lifecycle: active。
- Compatibility boundary: 只改变用量明细的 owner-facing 数值呈现；保留后端聚合、成本可用性、历史 unknown 和现有响应式语义。

## Replacements / Background

- 本主题承接 Dashboard 用量明细长数字在固定表格列中溢出的显示问题。
- 设计复用已有 `AdaptiveMetricValue` / `adaptiveMetricValueSpec` 体系，并将完整值提示收敛到项目自定义 `Tooltip`；不创建新的量级格式体系。
- 表头采用单行紧凑可见标签与完整无障碍名称分离的呈现方式，避免固定列宽下的相邻标题重叠；桌面和 `mobile390` 的已确认视觉证据保存在主题资产目录。
- 无需新增 ADR：该变更是可回退的前端呈现扩展，且不改变持久化或外部接口边界。

## Delivery Sync

- PR 交付分支已同步当前 `main` 基线；本主题文档继续覆盖该同步后的实现树与验收证据。

## Related Changes

- None

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
