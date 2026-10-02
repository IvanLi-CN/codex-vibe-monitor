# 模型管理与价格同步 主题历史

> 这里记录主题局部生命周期、兼容性与背景；完整取舍保留在 ADR 中。

## Lifecycle / Compatibility

- 旧版静态代理预置候选和已启用状态由一次性迁移导入持久模型目录；迁移完成后，重启不得重新加入已删除候选。
- 新表为增量 SQLite 状态。旧版程序不理解动态候选；旧版设置写入可能无法保留动态预置成员，回滚不删除迁移数据。
- 模型删除、手工编辑和价格同步均不改写历史非空调用成本。

## Replacements / Background

- 由已接受的 [ADR 0023](../../adr/0023-model-management-and-manual-price-synchronization.md) 建立。
- ADR 0018 中不提供外部价格目录同步的约束，仅由 ADR 0023 明确的用户触发、人工审阅流程取代。
- 同步勾选从自动选择无冲突的新模型与非手工价格变更，改为没有历史选择记录时默认不勾选，并恢复已有的选中及主动取消状态；供应商冲突仍须明确解决。
- [ADR 0026](../../adr/0026-service-owned-price-sync-memory.md) 将选择和模型发现记忆确认为服务实例共享的持久状态，放弃浏览器独立维护的方案。
- 选择变更立即保存，不等待价格应用成功；取消或关闭价格审阅不撤销已保存的选择。
- 新模型提醒改为初始完整目录基线之后的模型 ID 首次发现；实际看到模型行后记录查看，本次审阅保留小点，后续审阅消除，未查看的提醒持续保留。
- 模型批量选择限定为当前筛选后的全部结果，包括可视区域之外的匹配行；被过滤隐藏的选择保持不变。
- 审阅默认隐藏来源明确标记为已废弃的供应商报价，并提供显示开关；不依据发布时间或模型名称猜测下架状态，隐藏不删除本地价格或历史选择。
- 模型勾选按模型 ID 与供应商 ID 分别记忆，并恢复明确选择的报价供应商；切换供应商不继承另一报价的勾选，隐藏／缺失时不自动改选其他报价。
- 搜索继续只影响显示和批量操作范围；实际写入数量与长期勾选意愿分开，同价条目不重复写入，搜索外仍将应用的选择需要明确计数反馈。

## Related Changes

- Direct delivery: [PR #1064](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1064), based on `b68295595d875aead33ef31514eaf602279ece94`. Implementation and review-layout commits include `653e1cd8`, `ab6c8ca8`, and `3df25495`; current CI and formal review evidence are tracked on the PR.
- Compatibility classification: additive API and service-owned SQLite state, verified as `minor` in `assets/sync-memory-version-impact-record.json`; release labels are `type:minor` and `channel:stable`.

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
