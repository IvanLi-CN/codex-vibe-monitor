# 独立性能遥测历史

- 2026-10-08：连续多次 GitHub-hosted 测量显示负载期间 CPU/IO PSI 会升高，即使 1,500 个请求按时完成且 p95 稳定；保留严格的三次安静准入和首个窗口样本校验，将正式窗口压力改为 `pressureExceededSamples` 原始证据。只有结构性采集错误、首样本不满足准入或预算/稳定性失败才阻断经验性卡，避免把负载期间的真实资源争用误报为 collector 故障。
- 2026-10-08：补充稳定性失败的边界：窗口 CV 超过 5% 仍记录失败，不把它当作通过；只有两项指标的中位数观测增量仍在 5% 内且 trace 无丢失时，才允许以 `unavailable` 作为 hosted runner 环境限制收口。中位数预算超标、trace 丢失或证据缺失继续阻断。
- 2026-10-08：当前候选连续两个 GitHub-hosted A/B 首窗口在请求负载下触发 CPU PSI 超限；保持严格 unavailable 合同，测量脚本改为按模式停止未使用的 Prometheus/Grafana/Tempo/entry 服务，避免前置 trace WAL 和无关 dashboard 工作污染开销窗口，同时保留 metrics-only 与 trace 摄入的真实路径。
- 2026-10-08：服务 profile 隔离及负载并发收敛未消除 CPU PSI 超限；应用改为固定到启动时 runner affinity 中相对空闲的单核，保留严格 PSI 阈值和预算合同。
- 2026-10-07：当前候选将 GitHub-hosted 资源压力导致的性能证据 `unavailable` 与真实验收失败分开分类。辅助 job 保留失败证据和 `empirical-card.json` 的不可用状态，仅在默认 A/B 场景明确报告压力超限且其余场景通过时中性收口；预算超标、功能失败、采集器错误和证据缺失仍阻断。
- 主人明确要求性能实验只在 GitHub Actions 执行，不进入每个 PR 的必要测试集。
  PR #1079 将性能预算和 CPU 诊断保留为辅助检查，解除 Build Artifacts 对其结果的依赖，
  同步质量门禁合同与自测；5% 验收标准、失败证据及专项验证责任保留。
  历史开销超标和缺测不改写为通过，未完成的归档容量目标继续单独跟踪。

- 同步主线 `83436dd4` 的 invocation range、pending terminal identity 和新 ADR。
  入队观测时间与业务登记同时保留；主线长等待/恢复/旧读取验收迁入独立业务 harness，
  不恢复旧性能 SQLite 或自研图表。同步改变业务热路径，要求新候选完整验证与审查。

- Hosted run `37300494717` attempt 2 exposed import-time CLI parsing in `client.py`,
  making all CPU diagnostic counter snapshots `unknown`. Guard the dispatch with
  `__main__` and cover the real import path in a subprocess regression. This repairs
  diagnostic evidence collection; it does not explain CPU variance or certify the
  budget, and the new candidate requires fresh Actions and formal review evidence.

- 主线合并候选 `9d6258f0` 在 Actions 构建前被 `--locked` 拒绝：WS 依赖清理误删观测依赖仍需的 rand 0.9 传递依赖族。离线 workspace 解析补齐锁文件，直接依赖、性能阈值和普通修复批次数不变；未启动的预算/诊断不作证据。

- 候选 `9d06607c` 的 Actions run `37266146248` 完整 attempt 4 通过全部 20 个 job 与四项经验性场景，CPU/请求 +2.42%、p95 −0.34%。先前环境 unavailable 与部分重跑输入失败分别保留，不以通过结果宣称历史根因已解决。随后同步主线 `75de1168` 的 WS 退役、请求日志脱敏与任务标题对齐；新运行时和 render inputs 须重新绑定验收、任务截图与正式审查，普通修复批次数维持七批。

- 候选 `ccfd94b7` 的 Actions run `37225986112` 保留全部六窗与 9,000 个请求，开启组 CPU/请求 CV 为 11.77%，预算比较无效；p95 变化 +0.57%。与先前通过候选的单次差异核对仅发现退役恢复工具、回归与文档变化，无法据此确定根因。主人批准独立、有界的 Actions CPU 诊断，普通修复批次维持七批，预算规则不变，诊断不能签发通过或触发正式审查。诊断前正常合并主线 `7681ce2d`，保留任务工作量业务功能并退役冲突处旧性能摘要；新主线运行时变化必须在后续比较中明确。

- 候选 `36d8c612` 的 GitHub Actions run `37219321082` attempt 1 完成 18 个 CI job；六个测量窗口共 9,000 个请求，CPU/请求增加 1.48%、p95 增加 0.46%，六窗资源压力与证据完整性均通过。六条正式审查完成后，隔离复现确认恢复中断留下部分最终目标并阻断重试；主人批准第七轮有限修复。恢复改为同目录校验后原子禁止覆盖发布，支持正确 alias 与已发布目标重入，新增八项故障回归；32 项工具回归通过。该修订仍需新 SHA 的 Actions 验收和全部六条正式审查。初始准入与六窗额外等待边界按既定合同保留，相关审查意见记录为观察，不更改预算。

- 候选 `9369ec3e` 在 GitHub Actions run `37208191984` 完成 18 个 CI job，数值 CPU/请求变化 +3.33%、p95 +0.53%，9,000 个测量请求完成。六条审查后，隔离复现确认额外 CHECK 或不同 DEFAULT 仍被识别并退役；资源时间界限也确认测量窗口中的压力仅记录未判定，因此数值通过不能证明环境可归因。主人批准完整 DDL 核验与逐窗口压力判定规则，以及同一 PR 的第六轮有限修复；新合同与候选须重新验证。固定窗口与现有代理身份模型保留，实际入口头覆盖由 101 上线验收确认。

- 候选 `0f2c3ea6` 的 GitHub Actions run `37196758665` 完成全部经验性场景和 18 个 CI job，CPU/请求变化 +4.55%，p95 变化 -0.13%。六条正式审查随后确认浏览器同源/数值拒绝绕过配额；隔离 HTTP 复现中，同一客户端 120 次拒绝后仍有 30 次成功配额，全局拒绝也不消耗配额。经明确授权，将现有限流移至固定 POST 路由入口并补齐真实 HTTP 回归，修订候选需独立的新 Actions 验收与正式审查。

- 候选 `c3a2f45b` 在 GitHub Actions run `37190238804` 完成全部经验性场景和 18 个 CI job，CPU/请求变化 -1.27%，p95 变化 +0.37%。正式审查随后复现验证后新增 WAL 导致文件族拆分；经明确授权，退役工具补齐未登记成员拒绝检查并保留部分删除后的重入。14 项迁移回归通过，修订候选仍需独立的 Actions 验收和正式审查。

- 候选 `246a6b80` 的 GitHub Actions 再次完成全部经验性场景和 18 个 CI job，CPU/请求增加 3.83%，p95 变化 -0.17%。随后正式评审补齐资源采样错误状态；该修订需要独立的新候选验收，不能沿用上述通过结果。

- 正式评审修复 CPU 采样失败的观测降级状态，补齐 SQLite SQL 模板中的双引号、注释和数值形式脱敏；修复后重新绑定候选的 Actions 验收与六条正式评审，不复用旧 SHA 的通过卡。

- 主题建立于独立性能 SQLite、固定低基数维度和分层留存决策之上。
- 现有研究记录与 ADR 保留为背景；本主题负责长期契约、接口和实现边界。
- 代理请求阶段、结果、重试和字节数由业务主库调用明细负责；遥测库只保留主库没有的系统运行开销与浏览器体验趋势。
- 遥测库是可丢弃的附属状态。停止或损坏的旧版本由新版本前向修复或重新初始化，不回滚业务主库状态。
- 代表性负载的遥测开销 A/B 保留为发布后诊断项，不作为合并门禁；共享测试机的主库维护争用和不稳定基线必须与遥测结果分开记录。A7 容量、写锁隔离、故障恢复和直接质量门禁仍是交付要求。

- ADR 0025 替代 ADR 0019：Prometheus/Grafana/hotpath 接管，旧性能 SQLite、API 与图表退役。原有 13 个月在线历史和非门禁 A/B 仅描述切换前实现；新合同要求 30 天在线留存与通过 5% 开销验收。
- 性能预算验收转由 GitHub Actions 的独立 GitHub-hosted 测量 job 完成。本地与共享测试机用于功能、集成和诊断；旧 A/B 记录保留但不作为当前预算证明。
- 主线连续统计检查点验收迁入独立业务运行时入口，保留三轮、重启和旧版本读取验证；观测 A/B 仍限定 GitHub Actions。
- 记录页严格 E2E mock 补齐观测能力接口的禁用响应，保留未知 API 报错；桌面与窄桌面覆盖通过。

## Mainline release source alignment

- Mainline released v3.0.0 before this major/stable PR; the cutover now targets v3 → v4.
- The retired writer source is identical in v2.86.2 and v3.0.0; schema-v1 DDL stays frozen.
- Accept only a pinned v3 image and matching stopped writer; reject v2 direct skips and v4 sources before file operations.
- Update migration/SemVer/operations records and regression fixtures together; Actions and fresh Tier 4 evidence must bind the changed candidate.

## 请求诊断架构

主人确认服务端入口至响应 body 终结、关联异步落盘旁路、全部 HTTP 代理端点、正常容量内全量轻量记录及 24h 链路留存。ADR 0033 允许独立外部诊断 trace，ADR 0034 选定可共享的 Tempo 与 OTel/OTLP；Prometheus 保持聚合历史职责。实施与隔离验证由一个 PR 交付，正式环境部署、合并和发布不在该交付授权中；计划不写入主题 Spec。
