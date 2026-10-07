# 数据保留维护任务归属实现状态

## Current Status

- Implementation: 已实现；剩余验证与交付被 Agent VM 正常准入拒绝阻断。
- Lifecycle: active。
- 需求与 ADR 已确认；当前工作分支按批准计划实现同一 PR，交付停在 merge-ready。

## Implementation Coverage

- `REQ-RMO-001`, `REQ-RMO-005`, `REQ-RMO-006`：归档移除身份清理和 raw 孤儿预演，两个清理使用共享请求／执行入口；raw 自主 worker 与旧归档 worker 已退出。身份每轮检查最多 32 个对话和 32 个小时前缀，共享独立 2 秒工作预算；raw 保留原候选上限、遍历和释放协议。巡检、工作接续与安全重试独立记录。
- `REQ-RMO-002`, `REQ-RMO-003`, `REQ-RMO-004`：移除两项环境输入；新增控制默认启用与 300 秒巡检，事务初始化使用 `retention_maintenance_ownership_v1`，保留已有控制与计划。四项开关只控制自动请求准入，已接受手动请求与已准入有界执行保留，恢复从当前时间排期。物化在轮次准入固定控制，保留源代际和提交围栏。
- `REQ-RMO-007`, `REQ-RMO-008`, `REQ-RMO-009`：沿用归档源转换／统计失效／刷新入队事务；身份使用实际运行管理器和资源准入，新增小时 scope 以删除与游标同事务提交。查询超时后 SQLite 连接关闭完成前保留身份围栏；raw 保留隔离、文件身份、库存与 release-pending 保护。
- `REQ-RMO-010`：CLI 在业务初始化和启动恢复前选择在线请求或离线独占路由。运行态锁分别绑定规范化业务库与维护库路径；在线不初始化控制或回收活跃运行，离线只执行指定任务并在释放锁前关闭连接池。三个预演使用局部扫描状态，不改业务删除、正式游标或释放事实。
- `REQ-RMO-011`, `REQ-RMO-012`：新记录包含职责版本、scope 与 dry-run，计量分离调用行、对话、小时、文件及字节，缺测保持未知。新增目录和详情、自动触发文案、暂停时立即运行、归档关联入口及旧组合历史标识。资源等待与实际开始分开记录，保留旧历史口径。
- `REQ-RMO-013`：新增维护库重试状态表及业务小时清理 scope；既有迁移保持不变。九个发布镜像均已生成独立来源夹具并在记录的候选通过升级；最新物化开始状态修复已通过实际进程验证，最终工程验证尚待完成。

## Verification

- 已通过：命名 Rust 初始化／暂停／重试／身份分页和预演／跨进程运行态锁／查询超时连接关闭围栏回归，共 7 项；1 项子进程辅助测试仅由父测试调用。
- 已通过：Web 类型、lint、构建，最新主干完整 Web 单测 178 文件、1811 测试（6 跳过），相关 Storybook 72 项，以及桌面／移动端和 demo 路由 Playwright 12 项；均已在当前主干基线刷新。
- 已通过：九个已发布来源在二进制 SHA-256 `15945b3a53712b0d40fb806fe2b1a72561ad9c20dc7a1152873e5a35db285b11` 完成升级、三项只读预演、暂停后显式真实运行、身份保护、观测持久化及初始化中断前向恢复，见 [升级结果](../../adr/assets/retention-maintenance-task-ownership/released-state-upgrade-results.json)。后续物化开始状态修复仍须刷新适用验证；此结果不代表最终交付就绪。
- 已通过：修复默认线程栈下归档调用链溢出和手动物化开始状态后，实际服务／CLI／页面进程竞争验证通过，见 [运行态结果](../../adr/assets/retention-maintenance-task-ownership/runtime-validation-results.json)。覆盖维护库不可用、不执行启动恢复、四项暂停后手动运行、不安排自动接续、重复准入保护和退出释放双运行态锁。
- 被阻断：共享 backend runner 的三个顺序资源 profile、最终 Rust 格式／check／clippy 和最终候选全部九来源重测。VM 续约被 `draining` 拒绝后于 2026-10-07T06:23:28Z 到期停止；正常恢复同一 VM 也返回 `draining: new leases are refused`。最终候选已取回 v4.0.0–v4.0.2 的完成结果，未完成来源和重型检查不作通过处理。
- `VER-RMO-001` 至 `VER-RMO-008` 的完整证据映射待当前候选验证结束后同步；当前无正式 review 或 PR Ready 结论。恢复锚点及已通过／未通过门禁见 [当前候选验证卡](../../adr/assets/retention-maintenance-task-ownership/current-candidate-validation.json)。

## Visual Evidence

来源：mock-only `ui_demo`；桌面 1280 CSS px，移动端由 demo iframe 固定为 393×852 CSS px。六张图片在锁定基线不存在同路径旧图，按 current-only 比较展示后已获主人明确确认；无需裁剪，不使用生产数据或真实程序截图。

![桌面任务目录](assets/catalog-desktop.png)
![桌面调用身份清理](assets/identity-desktop.png)
![桌面 raw 孤儿文件清理](assets/raw-desktop.png)
![桌面归档关联入口与旧历史](assets/archive-desktop.png)
![移动端暂停后手动运行](assets/identity-mobile.png)
![移动端独立任务目录](assets/catalog-mobile.png)

## Rollout and Remaining Gaps

- 公开配置移除、一次性入口范围及自动触发控制语义按 breaking／major 记录；持久状态影响单独评估，不由公开合同的 Major 自动决定。
- 直接来源逐一为 v4.0.0、v4.0.1、v4.0.2、v4.0.3、v4.0.4、v4.0.5、v4.0.6、v4.1.0 与 v4.1.1；夹具分别由发布镜像生成，来源与镜像摘要保存在 [released-state-sources.json](../../adr/assets/retention-maintenance-task-ownership/released-state-sources.json)。尚未声明最终候选升级验收通过。更早 Major 必须先中间升级，不支持多版本同时写入。
- 实现从计划基线 `130c357c043a3449512a25cd82643fc5a8ca02df` 开始；主干合入对话统计变更后按已授权的同步合同对齐当前主干，保留实现备份并刷新验证与审查。
- 必须完成最终候选工程检查、发布来源升级、Tier 4 只读审查及同一 PR 当前 head 的必要 CI，才可宣告 Step 5C Ready。

## References

- [Requirements](SPEC.md)
- [Background](HISTORY.md)
- [Ownership decision](../../adr/0033-retention-maintenance-task-ownership.md)
