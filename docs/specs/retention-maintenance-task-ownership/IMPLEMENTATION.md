# 数据保留维护任务归属实现状态

## Current Status

- Implementation: 已实现；PR #1092 的 CI 修复与主干同步正在进行，本会话 VM 已在容量释放后恢复，最终验证与交付尚未完成。
- Lifecycle: active。
- 需求与 ADR 已确认；当前工作分支按批准计划实现同一 PR，交付停在 merge-ready。

## Implementation Coverage

- `REQ-RMO-001`, `REQ-RMO-005`, `REQ-RMO-006`：归档移除身份清理和 raw 孤儿预演，两个清理使用共享请求／执行入口；raw 自主 worker 与旧归档 worker 已退出。身份每轮检查最多 32 个对话和 32 个小时前缀，共享独立 2 秒工作预算；raw 保留原候选上限、遍历和释放协议。巡检、工作接续与安全重试独立记录。
- `REQ-RMO-002`, `REQ-RMO-003`, `REQ-RMO-004`：移除两项环境输入；新增控制默认启用与 300 秒巡检，事务初始化使用 `retention_maintenance_ownership_v1`，保留已有控制与计划。四项开关只控制自动请求准入，已接受手动请求与已准入有界执行保留，恢复从当前时间排期。物化在轮次准入固定控制，保留源代际和提交围栏。
- `REQ-RMO-007`, `REQ-RMO-008`, `REQ-RMO-009`：沿用归档源转换／统计失效／刷新入队事务；身份使用实际运行管理器和资源准入，新增小时 scope 以删除与游标同事务提交。查询超时后 SQLite 连接关闭完成前保留身份围栏；raw 保留隔离、文件身份、库存与 release-pending 保护。
- `REQ-RMO-010`：CLI 在业务初始化和启动恢复前选择在线请求或离线独占路由。运行态锁同时绑定规范化数据库路径对、当前数据库 inode 对与两个数据库文件本身；在线请求必须确认路径对和当前 inode 的 ready 证明一致，初始化、离线 CLI、混配数据库对、替换 inode 与 hard-link 别名均在打开 SQLite 前拒绝。离线多链接数据库返回不可用，避免不同日志路径写同一数据库。在线不初始化控制或回收活跃运行，离线只执行指定任务并在释放锁前关闭连接池。三个预演使用局部扫描状态，不改业务删除、正式游标或释放事实。
- `REQ-RMO-011`, `REQ-RMO-012`：新记录包含职责版本、scope 与 dry-run，计量分离调用行、对话、小时、文件及字节，缺测保持未知。新增目录和详情、自动触发文案、暂停时立即运行、归档关联入口及旧组合历史标识。资源等待与实际开始分开记录，保留旧历史口径。
- `REQ-RMO-013`：新增维护库重试状态表及业务小时清理 scope；既有迁移保持不变。直接升级来源逐一覆盖 v4.0.0 至 v4.0.6、v4.1.0 至 v4.1.3、v4.2.0 与 v4.2.1，不以版本等价假设共用来源。最终候选的十三来源升级、进程安全验证和 Tier 4 复审尚在收敛。

## Verification

- 当前源码输入：Rust 树 `95af4b60a35ba18d947ee3c377f581d697448d61`，Web 树 `e352311b912402a2bd869630dd5c993fc454bc63`；260 个 Rust／构建输入已在会话 VM 逐项核对，指纹为 `c27f83124cd0a41a0d480dd23ce7b38966a9a9eb2767ef8f87e40121fa37faf9`。
- 已通过：Web 类型、lint、构建；完整单测 179 文件、1838 测试（6 跳过），相关 Storybook 3 文件／72 测试，桌面／移动端与 demo 路由 Playwright 12 测试。测试夹具预先加载懒模块并等待实际渲染状态，未改变产品交互或生产阈值。
- 已通过：会话 Linux VM 中共享 runner 的 lightweight 组 1302/1302，源码质量门禁、质量门禁合同与后端测试脚本合同。SQLite 与 archive/file-I/O 组、Rust fmt/check/clippy 的当前结果待补充。
- Linux 计时诊断：增加 vCPU 和 nextest 独占执行槽均未消除共享宿主资源下的既有 projection 计时失败；将测试进程及子进程固定到单个来宾 CPU 后，11 个原失败测试全部通过，lightweight 完整组通过。保留失败日志，所有原断言和生产时限不变；当前三组完整回归按相同执行方式顺序运行。
- 运行态修复统一路径／inode 锁排序，在 SQLite 初始化前持有实际数据库文件锁，并发布初始化、CLI 与 ready 角色。新升级与服务／CLI／页面竞争结果必须绑定最终候选二进制 digest；旧结果只保留为历史，不充当当前证明。
- 六张 mock-only 页面图片已获主人确认；主干合并后再次核对任务行、控制、计划、计量及详情，图中相关页面行为与呈现保持一致。新主干的统计公平队列、分页及实时 overlay 合同保留。
- `VER-RMO-001` 至 `VER-RMO-008` 的实现证据映射到 [当前候选验证卡](../../adr/assets/retention-maintenance-task-ownership/current-candidate-validation.json)。当前尚未满足 PR Ready；必须完成十三来源升级、进程验证、Tier 4 审查及 PR 当前 head CI。

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
- 直接来源逐一为 v4.0.0 至 v4.0.6、v4.1.0 至 v4.1.3、v4.2.0 与 v4.2.1；各来源由对应发布镜像独立生成，来源与镜像摘要保存在 [released-state-sources.json](../../adr/assets/retention-maintenance-task-ownership/released-state-sources.json)。更早 Major 必须先中间升级，不支持多版本同时写入。
- 实现从计划基线 `130c357c043a3449512a25cd82643fc5a8ca02df` 开始，按已授权同步合同保留备份并对齐主干；已发布后仅使用签名合并提交，当前基线为 `ac9bdfda97c79c666448db2b38eb6131d2c92d15`，保留主干统计公平队列及其回归，不重写已发布历史。
- 必须完成最终候选工程检查、发布来源升级、Tier 4 只读审查及同一 PR 当前 head 的必要 CI，才可宣告 Step 5C Ready。

## References

- [Requirements](SPEC.md)
- [Background](HISTORY.md)
- [Ownership decision](../../adr/0034-retention-maintenance-task-ownership.md)
