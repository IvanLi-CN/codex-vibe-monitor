# Stats 长期用量与性能统计实现说明

## 当前实现边界

- 后端持久化、回填、soft-delete、overview/series API 与前端长期统计区均以 `5k89c/SPEC.md` 为契约。
- 运行态 terminal 通过 projection cursor 增量写入热小时/自然日；每 60 秒只合并新 terminal 与明确 dirty bucket。全量 `refresh_long_term_stats` 仅保留给首次无 durable baseline 的准备阶段，不是生产周期性路径。
- 初始 refresher 与直接 refresh 入口都以 pressure/cancel-aware 的 `512` 行微事务替换 rollup 和 replay marker；已有 `ready` 汇总的 refresh 先切换每个日期的 durable last-good snapshot，因此 API 在完整替换前持续返回旧日。持久 `error` 或初始 incomplete marker 保持为可重试初始物化，即使此前留下部分日汇总。daily verification 只在对应日 repair、retention 与状态写入完整成功后写入 durable completion，legacy fallback 会过滤 rebuild suppression。
- 现有 Stats 查询、筛选、bucket、SSE 与图表路径保持不变；长期区使用独立 endpoint、hooks 和组件状态。
- 新增 schema 必须兼容旧 SQLite：启动迁移可重复执行，回填状态可恢复，archive purge 只有在长期汇总 target materialized 后才可继续。
- 完整性审计和修复只在长期统计链路运行：初始全量和增量候选日/小时都要先对照 `invocation_rollup_hourly` 的可信终态 overall 证明，再在单个事务内替换所有维度；有界 canonical 增量一律保持未证明，只有完整来源对账才能标记可信；修复队列持久化检测、重试时间和失败原因。archive manifest 与 live 调用必须在同一 SQLite 读取快照枚举，避免 retention 交接被认证成完整空洞。
- 对账会先校验 completed 调用 archive 的文件 SHA-256；manifest 不匹配即按不可用来源处理。归档清理把无法立即跨越的安全下界持久化为 pending 候选，只有全部仍保留的调用/请求尝试来源都位于候选之后，才提交连续来源边界。旧 schema 增加终态证明列时，既有 canonical 历史先被固定在升级当天之前的迁移下界；该下界字段同时是迁移完成标记，因此即使所有 `ALTER TABLE` 已执行后进程中断，下次启动也会补齐边界。缺失的调用或请求尝试 archive 均保留 manifest，不能被误认作成功清理。

## 计划落点

- `src/schema.rs`：长期小时/日汇总、回填状态和 `pool_upstream_accounts.deleted_at`。
- `src/maintenance/`：live/archive 回填、materialization marker、小时 retention 与日汇总确认。
- `src/api/slices/long_term_stats_api.rs`：overview/series 读取合同和参数校验。
- `src/upstream_accounts/`：API Key 软删除及账号池/同步/路由隐藏。
- `web/src/features/stats/`、`web/src/hooks/`：独立长期区、图表、虚拟化表格与数据 hook。
- `web/src/demo/` 与 Storybook：mock-only 整页场景和可复用片段状态画廊。

## 本次升级

- `LongTermChart` 增加明确的 `line` / `stackedArea` 模式；模型用量和上游账号的 Token、成本、调用次数均使用绝对值堆叠面积。
- 堆叠数据以 `overview.daily` 的完整日期窗口补齐；缺失 point 和已有 `null` 指标均写为零值，使数据岛之间保持连续零基线。自定义 tooltip 同时展示各系列和当日总计，并按 overview 顺序稳定图层、图例和 tooltip；折线图继续保留原始缺失值语义。
- 模型表新增 sticky 全量总计行，身份列改用 `ModelPerformanceModelIdentity`，模型行收紧至约 `40px`，搜索不影响总计。
- Storybook ready fixture 与 mock-only demo fixture 使用独立的模型思考程度字段，覆盖桌面/移动状态和关键交互。
- Storybook `SparseSeries` 与 mock-only demo 的稀疏长期序列覆盖数据岛场景，作为连续零基线的可视化回归入口。
- 全局趋势以当前选择指标对应的序列名驱动 Recharts 图例和 tooltip：Token、成本、调用次数分别显示各自的总计标签，避免折线数值与“全部调用”混淆。
- `LongTermSeriesVisual` 将长期多序列图的颜色、面积透明度、线型和完整标签集中到稳定映射：完整模型名决定家族色，思考程度决定明暗与虚线；账号序列独立分配八色分类色板。图表、tooltip、图例和已选表格行复用该映射。
- 长期图例改为固定可读的自定义内容：已识别模型以图标替代重复名称并保留思考程度，无图标模型和上游账号继续显示完整名称；完整模型名通过原生悬浮提示与辅助文本提供。色标、图标和文字使用同一居中基线。Storybook 的 `SeriesIdentity` 与 mock demo 提供八项模型/账号高密度状态。
- 虚拟化表格的表头、总计行和数据行统一使用块级 flex 冻结列，避免身份列与绝对定位的指标虚拟列发生行内基线错位。
- 表格沿用 `ModelIdentity` 的图标优先合同：已识别模型以图标取代可见名称，完整名称通过原生悬浮提示与辅助文本提供；未识别模型保留文字名称。

## 验证记录

- 已落地 `src/long_term_stats.rs` 的三维小时/日汇总、可恢复 live/archive 回填、准备状态进度、overview/series API 与墙时区间并集。
- 已落地 `pool_upstream_accounts.deleted_at` 迁移、API Key 凭据/会话/路由运行状态清理，以及账号池/路由候选隐藏。
- 已落地独立 `LongTermStatsSection`、60 秒可见刷新 hook、mock demo handler、Storybook ready/preparing/empty/error 状态与关键 play。
- 已落地长期统计完整性修复：不完整重建会同时从 partial/rebuilt 候选移除；调用归档物理删除前会扫描真实调用，且每条参与来源边界的调用与请求尝试匹配来源都必须能解析为有效终点，任一异常行都会保留归档。旧 archive 缺失可选 `invoke_id` 或来源时间列时按 `NULL` 读取；canonical 证明重放也会将缺失的任意可选调用字段（包括全部时间列）读取为 `NULL`，并将缺失 `detail_level` 视为 `full`。删除先将候选来源安全下界与 `cleanup_state=delete_pending` 持久化在 manifest，归档保持 `completed` 可读；仅在文件删除成功或确认不存在后的最终事务中才推进全局来源下界并删除元数据，失败时保留 `delete_pending` 供后续重试。下界之前的 canonical 桶属于已验证但不可再重建的历史证据，不会因故意退役而撤销证明；下界当日及之后由完整来源严格校验。未进入 `delete_pending` 的已物化调用 archive 若在来源可用性审计中确认缺失，会保留 manifest 作为持续的不可用来源证据；对账同时清除该 archive 的长期 replay marker，确保同身份的恢复文件会在下一次刷新重新读取。请求尝试归档须先把账号映射追溯到可读调用来源，无法复核时不删除归档；repair queue 在确定跨午夜前置日期后的最终重建范围内加载请求尝试 archive，任何 completed attempt archive 不可读都会阻断替换并使 API 保持 `error`，避免将可恢复的历史账号维度降为 `other`。暂时不可读的调用归档仅在存在有效 `coverage_start` 时从该日阻断候选 UPSERT；起点缺失时从整个保留窗口下界阻断，保留现有行并返回既有 `error` 状态。canonical 小时表新增与长期口径一致的终态调用/Token/成本证明及 `terminal_proof_complete`。有界增量写入会清除可信标记；所有已完成 archive 和 live 来源均可读时，审计扫描会原子回写与来源不一致的 canonical 桶，并删除可重建窗口内已不在来源扫描结果中的陈旧桶，随后按上海日期重建长期汇总（支持已验证空结果）。只有 archive 缺失或不可读、必需源表/列缺失、或日/小时证明无法建立时，才撤销证明、保留既有长期行、写入 repair queue 并保持 `error`；这类错误会在每次后续刷新立即重试。canonical 空日会清理任一维度的非零残留，同时保留零调用、零 Token、零成本的合法墙时续段。无法证明的已排队日期保留既有行并写入持久化退避，在可执行队列清空前使 API 持续返回现有 `error` 状态；SQLite 锁仅在该后台链路按 `250ms/1s/3s` 有界重试。
- 删除收尾以 `id + dataset + 路径 + SHA-256 + delete_pending` 作为 CAS 身份，并在 `BEGIN IMMEDIATE` 中锁定该身份再删除文件与元数据；legacy writer 在同一 SQLite 写入区间内重新激活 manifest 并替换文件，避免重写与收尾竞态删除新归档。
- 已通过：前端 `bun run test`（1310 passed / 6 skipped）、目标组件 Vitest、`bun run build` 与 5 个变更文件 Biome 检查；根级 `lint:web` 仍有既有无关文件错误，未扩大范围修复。
- Storybook interaction/a11y、mock-only `ui_demo` 桌面/移动视觉证据及最终截图 SHA 在本次收口阶段补录到 `SPEC.md` 的 `## Visual Evidence`。
- 已通过：长期系列视觉解析器 Vitest（7 assertions），专用 Biome 检查，`bun run build`、`bun run demo:build`、`bun run test-storybook` 与 `bun run storybook:build`；全仓 `lint:web` 仍有既有无关诊断，未扩大范围修复。

## Memory Attribution

- 长期 projection flush 在读取现有 runtime/interval 状态前后记录进程 RSS、`VmHWM` 和已知组件估算；日志明确区分 `retained_bytes`、`retained_delta_bytes`、`peak_delta_bytes` 与 `load_row_count`。
- interval index 的估算只读取现有内存容器的长度、容量和字符串容量，不复制区间或重算数据库数据。该指标用于归因，不作为回收或限流依据。
