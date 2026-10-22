# 数据保留维护任务归属背景

## Lifecycle / Compatibility

本主题建立四项维护的独立执行归属与自动触发控制。既有任务标识、归档 URL 和旧组合历史继续可读；新增清理任务使用独立标识。`RETENTION_ENABLED` 和 `XY_RETENTION_ENABLED` 完全退出配置输入，不做停用状态兼容导入，因此公开配置合同存在行为变更。

任务停用在本主题中表示暂停自动触发；显式手动运行仍允许，已开始的有界运行保持正常范围。该语义接管这四项任务的旧“禁用后拒绝立即运行”或“禁用后截断当前轮次”规则。新控制默认允许自动触发，已有持久控制和管理员计划保留。

## Replacements / Background

- 主人已确认完整需求集合，关联 ADR 已接受；已按主人明确批准的实现计划进入实现与验证。
- [Bounded retention runs](../bounded-retention-runs/SPEC.md) 继续拥有归档预算、正确性和资源合同。本主题接管其中会话派生维护的归档阶段归属、跨任务组合完成度和自动触发暂停的含义；不整体取代该主题。
- [Invocation identity](../proxy-invocation-identity/SPEC.md) 与 [raw retention recovery](../autonomous-retention-recovery/SPEC.md) 继续拥有身份和文件删除安全。本主题调整执行者归属，不放宽保护条件。
- [ADR 0034](../../adr/0034-retention-maintenance-task-ownership.md) 保存四项拆分、弃用环境输入、默认值、调度和 CLI 范围的取舍；词汇由根目录 `CONTEXT.md` 定义。

## Related Changes

- 文档在签名提交中保留；实现位于 `th/retention-maintenance-boundaries`，同一 [PR #1092](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1092) 交付至 merge-ready。
- 主人明确继续解决审查问题。第十三批修复补上归档 owner 失败后的有界 catch-up 重试，租约冲突的 `failed` 与 owner v2 事实、`task_execution_active` 重试原因，以及测试目标所需的显式 `FutureExt` 导入；本地格式、差异和 JSON 检查通过，PR #1162 的新 head CI 与五 lane 正式复审待完成。
- 主人继续授权解决 exact-head CI 问题。第十四批修复将迁移夹具从旧的 schema 隐式清理预期更新为保留 205 条工作量样本，并移除 owner catch-up 更新中的无必要借用；本地格式与差异检查通过，代码候选为 `cb4c1bef`，新的 exact-head CI、迁移回放和五 lane 正式复审待完成。
- 主人继续授权解决第十五批问题。五 lane 正式复核确认模型路由剩余量、历史清理部分提交事实、重试计划写入、旧 finalizer 清理新 catch-up、实际工作观测、压力退避优先级、v2 旧版范围标签及归档批次事实的 P2 缺口；随后 stateful CI 暴露暂停测试未覆盖新增六个 v2 owner。第十五批候选 `cfe996f5` 已保留已提交事实、增加运行代际保护、补齐 v2 批次与观测合同，并扩展测试范围；本地格式与差异检查通过，新的 exact-head CI、迁移回放、五 lane 正式复审和 UI 视觉证据待完成。
- 直接兼容验证按独立发布来源执行；覆盖 v4.0.0–v4.0.6、v4.1.0–v4.1.3、v4.2.0 至 v4.2.2，不依据格式猜测合并来源，最终候选结果见 IMPLEMENTATION.md。
- PR Linux CI 暴露首次启动后 inode 补锁排序与路由不一致，修复统一排序，并保留初始化／离线 CLI 角色，避免数据库对锁提前发布 ready；新增 raw dry-run 回归移至独立批次测试文件，遵守既有源文件预算。

- 后续修复在 SQLite 打开前取得实际数据库文件及规范化路径／inode 对锁；在线准入同时验证 ready 角色、路径对与当前 inode，拒绝日志路径不确定的 hard-link 别名和离线多链接数据库。
- 同步主干的分页、实时 overlay 与统计公平队列；保留公平准入、源围栏、诊断及对应测试，仅按已批准合同固定维护轮次准入后的停用语义。
- 主干新增另一份 ADR 0033 后，本任务已批准的决策文件仅顺延为 ADR 0034 并更新引用，决策正文未变更。
- 正式并发审查发现既有 inode 对锁的快速返回遗漏路径／描述符重查，且 SQLite 随后可能打开替换后的文件；主人明确授权第四批修复。修复保留现有运行态独占合同，将校验接到 SQLite 原生打开边界，回归覆盖业务库、维护库、路径恢复后的不同原生句柄、后续连接、归档 ATTACH、硬链接和锁释放。

- 主人明确要求解决后续审查问题，第六批修复在维护库初始化失败时等待连接池关闭，并将路径与 inode 证明延伸到已打开 SQLite 文件的读写、日志及共享内存回调；替换文件保持原样，mmap 禁用以使访问经过受保护回调。新实例每取得一个协议锁即撤销旧 ready 标记，避免尚未完成锁集合时接受在线请求。原生句柄保留各自布局与方法表，归档 ATTACH 继续使用原生行为；职责、状态格式与自动触发合同不变。

- 当前 PR 的 CI 在上海午夜后一分钟暴露既有统计夹具的自然日预期错误：两分钟前的记录已属于昨天，`today` 仍期待两条。按主人要求修复两个测试的日期与 token 计数预期，保留滚动窗口及全量断言；通过进程内持续推进的 realtime 偏移复现原失败，并验证午夜前后及跨午夜。生产代码、职责、删除边界和持久状态合同均未改变。

- 主人明确 `RETENTION_ENABLED` 及其旧别名是临时功能，不要求兼容，并确认 Minor／stable 继续合并。版本影响重新按支持范围评估：旧部署文档承诺归档操作，未承诺附带孤儿清理；现有命令、任务标识、URL 和历史保留，新清理入口为增量能力。修正先前将临时配置退出自动归为 Major 的判断，执行职责、控制语义和持久状态合同不变；同步迁移证据中的当前版本分类，保留其通用跨 Major 升级约束。

- 主人继续授权解决第十六批问题。对 1371a7da 的正式复核发现独立详情裁剪绕过分钟聚合覆盖门禁、重试事实写回存在编译错误、证明加载取消可能误报完成、失败退避可被正常周期绕过、旧 finalizer 缺少代际保护，以及 shutdown quarantine 丢失完整 observation；第十六批候选 95b95759 已补齐这些边界，并把暂停回归扩展到全部新增 v2 owner。旧头三次 exact-head CI 运行均失败；新头的 exact-head CI、迁移回放和 fresh formal review 仍待完成。
- 主人继续授权解决第十七批问题。对 026e37d9 的 fresh 五 lane 复核发现重试排期的 move 后借用编译错误、同步归档 I/O 缺少外部 no-progress watchdog、后续批次失败丢失 raw 文件删除计量、安全退避被 work_remaining 清理、租约冲突排期失败无法重放重试意图，以及 dry-run 归档产生一秒循环接续；第十七批候选 f28248f6 已修复这些边界。当前候选的 exact-head CI、迁移回放和新的 fresh formal review 仍待完成。
- 主人继续授权解决第十八批问题。对 f28248f6 的 fresh 审查发现 watchdog 错误分类、native worker 排空、历史解压进度、retry/finalization 调度窗口、失败 hasMore 事实、dry-run 文件计量和 dry-run owner catch-up 清理缺口；第十八批候选 370be26c 已修复这些边界。当前候选的 exact-head CI、迁移回放、fresh formal review 和 dry-run UI 视觉证据仍待完成。
- 主人继续授权收敛主干同步后的当前 head。主干将 observability acceptance 改为 serial bundle 后，五 lane formal review 在 `3508ab41` 发现 candidate-source drift 夹具漏复制 ADR 0047；第十九批修复以 `27eb5b13` 补齐复制清单。当前 head 的 CI PR run `39760631680` 全部通过，五 lane 正式复审 clear；迁移回放和当前 head dry-run UI 主人确认仍待完成。

- 主干新增 schema startup phase 拆分 `e956d1a8` 后，签名合并提交 `e2144679` 将最终 PR head 对齐最新可信基线；CI PR `39764651636`、Docs Pages `39764651482`、Review Policy `39764651536`、Label Gate `39764651670` 全部通过，五 lane 正式复审绑定 `e2144679` 且无可执行 P0-P2 findings。迁移回放和 current-head dry-run UI 主人确认仍待完成。

## References

- [Requirements](SPEC.md)
- [Implementation coverage](IMPLEMENTATION.md)
