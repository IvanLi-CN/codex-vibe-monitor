# 数据保留维护任务归属实现状态

## Current Status

- Implementation: 未开始。
- Lifecycle: active。
- 需求与关联 ADR 已获主人确认。
- 当前仓库仍执行旧组合维护合同；需求文档不表示功能已经落地。

## Implementation Coverage

- `REQ-RMO-001`, `REQ-RMO-005`, `REQ-RMO-006`：现有归档和物化已有任务身份，身份清理仍嵌入归档，raw worker 尚未独立纳管；新任务归属和接续尚未实现。
- `REQ-RMO-002`, `REQ-RMO-003`, `REQ-RMO-004`：现有任务控制、默认值和配置读取需要按新语义调整；环境开关尚未移除，页面请求仍可能因任务停用而被拒绝。
- `REQ-RMO-007`, `REQ-RMO-008`, `REQ-RMO-009`：归档证明、刷新队列、身份围栏、raw 隔离和恢复协议已有基础；拆分后的保留性尚未验证。
- `REQ-RMO-010`：现有 CLI 仍含组合范围，直接调用底层函数而未共用任务请求、单实例与运行观测，部分 dry-run 推进正式孤儿游标；独立入口和预演合同尚未实现。
- `REQ-RMO-011`, `REQ-RMO-012`：旧三阶段、组合完成度、归档事件映射和历史展示需要接管；新旧范围标识及独立详情尚未实现。
- `REQ-RMO-013`：控制初始化与兼容记录为设计输入，未执行来源版本或中断发布验证。

## Verification

- 文档验证：使用 `dprint`、Spec 结构合同检查器、Markdown 链接目标及 JSON 解析检查。
- 行为验证：`VER-RMO-001` 至 `VER-RMO-008` 均尚未运行；没有代码、性能、迁移或界面验收结论。
- 实现阶段按仓库的最小完整验证路径选择对应 Rust 资源桶与 Web 验证；重型验证和 UI 证据遵循届时适用的仓库合同。

## Rollout and Remaining Gaps

- 公开配置移除已按计划记录为 breaking／major；持久状态影响单独记录，尚未验证。
- 受支持的已发布来源范围和候选版本身份必须由实现与交付阶段据实声明。
- 控制初始化不得覆盖已有选择；环境旧值不导入，新清理任务默认自动触发。
- 共享理解已确认，关联 ADR 已接受；实现与交付保持未开始。

## References

- [Requirements](SPEC.md)
- [Background](HISTORY.md)
- [Ownership decision](../../adr/0033-retention-maintenance-task-ownership.md)
