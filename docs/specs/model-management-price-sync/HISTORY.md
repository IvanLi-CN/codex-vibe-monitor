# 模型管理与价格同步 主题历史

> 这里记录主题局部生命周期、兼容性与背景；完整取舍保留在 ADR 中。

## Lifecycle / Compatibility

- 旧版静态代理预置候选和已启用状态由一次性迁移导入持久模型目录；迁移完成后，重启不得重新加入已删除候选。
- 新表为增量 SQLite 状态。旧版程序不理解动态候选；旧版设置写入可能无法保留动态预置成员，回滚不删除迁移数据。
- 模型删除、手工编辑和价格同步均不改写历史非空调用成本。

## Replacements / Background

- 由已接受的 [ADR 0023](../../adr/0023-model-management-and-manual-price-synchronization.md) 建立。
- ADR 0018 中不提供外部价格目录同步的约束，仅由 ADR 0023 明确的用户触发、人工审阅流程取代。

## Related Changes

- None. Record PR, commit, review, and compatibility references here; do not add task history to `SPEC.md`.

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
