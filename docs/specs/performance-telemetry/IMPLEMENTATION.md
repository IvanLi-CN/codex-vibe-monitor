# 外部性能观测实现

## Hosted CPU diagnosis

诊断快照 helper 的 CLI 分发仅在直接执行时运行，CPU diagnosis runner 导入
`client.py` 时不会因缺少 `mode` 丢失计数器。子进程回归覆盖无参数导入和固定
数值指标读取，保留既有 CLI 模式。此修复只改善诊断证据完整性，不改变六窗口
性能验收、5% 稳定性/开销阈值或预算结论；新候选仍须独立通过 Actions 验收。

候选 `ccfd94b7` 在 Actions run `37225986112` 的开启窗口 CPU/请求 CV 为
11.77%，超过 5% 稳定性上限；该数值不是已接受的 CPU 开销百分比。先前同合同
候选 `36d8c612` 通过，仅作为历史比较。普通修复批次停在七批，根因尚未确定。

独立 GitHub-hosted VM 的 `Observability CPU Diagnosis` 仅用于明确列名的 PR
#1071 和 #1079。#1079 的同合同候选 CPU/请求开销为 8.50%，超过 5% 门槛；
主线历史证据为 3.02%。现有采样发生在 A/B 之前，不能归因测量窗口内的差异，
因此临时复用该作业补齐窗口计数和 CPU profile；根因未定，不开启普通修复批次。
入口在 `scripts/observability-diagnostics/`，不修改预算验收脚本、场景摘要来源或
阈值；消费同一不可变镜像，复用合成初始化、60 秒预热与六个交替 300 秒窗口。
每秒记录绑定原容器和 PID start ticks 的 user/system CPU、throttle、context switch、
进程 IO，以及可获取的 runner frequency/steal。缺测保留 unknown；初始数据副本
必须具有相同有界指纹，指标只输出固定白名单数值之和，不保留动态标签。
开启和关闭模式的每个窗口均在负载开始 60 秒后采样一次 100 Hz/30 秒原实例
profile，验证 revision、container、build ID、样本数及产物哈希。任一模式缺少
profile 时诊断不完整；两组产物共同遵守原有容量上限。驱动清理仅使用本次创建
返回的 ID。
诊断采集会改变成本，`diagnostic-card.json` 永远不签发预算；即使并行的正常
验收通过，也不能在根因未明时宣称旧回归已解决，不自动重跑或开启正式审查。

#1079 的 `bfc0d23f` 在 Actions run `37436462176` 第二次执行中取得六个有效、
稳定窗口，CPU/请求相对开销为 5.61%，仍高于 5%；p95 相对开销为 0.64%，通过。
同镜像诊断完成，但只有开启模式的三个短 profile，无法直接识别新增开销。
补齐关闭模式的同阶段采样只用于归因；不修改预算合同、验收场景或阈值，不构成
普通修复批次，也不证明归档容量目标或旧回归已解决。

诊断前同步主线 `7681ce2d`，保留新的任务工作量趋势、业务状态与 SSE；移除合并
冲突中的旧性能摘要，保留 Grafana 任务链接。主线业务运行时发生变化，新的诊断
与旧失败候选只能用于带此限制的比较，不能把差异归因于退役恢复脚本。
随后同步主线 `8f9133dd` 的时间序列模块拆分；该次同步未改变 Web render inputs，
已确认的四张截图仍适用。预算合同与验收场景摘要均保持原值。
Agent VM 分配因 CPU/RAM 容量不足被拒；当前轻量工具回归与 mock 检查在本机完成，
全量工程门禁及性能实验交由本次自动 Actions。历史候选与当前工作树的证据分开绑定。
主线同步后的四张 mock 入口、移动、未配置与任务时间趋势截图已直接展示并获确认，
规范化未裁剪；桌面 CSS viewport 1280×900，移动 393×852。任务页保留业务运行
趋势与固定 Grafana 深链接，不再出现旧性能库摘要。

## Current candidate freshness

主线 `83436dd4` 合入 invocation range 与终态身份登记；合并保留入队时间和
pending identity 两条路径，并把主线长等待预热、cold refusal、range/recovery 与
older-reader 验收转移到独立业务 harness。旧性能库测试入口仍保持退役后的 runtime
适配器。此业务热路径变化要求新候选的完整 Actions 和六条正式审查；Web render
inputs 未变，已确认的四张入口截图仍适用。

候选 `9d06607c62c982ac8981ff1beaedc626df74f01e` 的 [Actions run 37266146248 attempt 4](https://github.com/IvanLi-CN/codex-vibe-monitor/actions/runs/37266146248/attempts/4)
全部 20 个 job 通过，四项经验性场景和六个稳定窗口通过：CPU/请求增幅 2.42%，
p95 变化 −0.34%。合同摘要为 `79e3689408855ab7c224994ac927bd0f4ea9602b1c943dcc974e2ed1dc5fc647`，
场景摘要为 `8217b5a3ea4252551e07391a48e570141d2e2047c1f5a520d282fcd016528f8d`。
attempt 1 的环境压力 unavailable 保留；attempt 2/3 在测量前因部分重跑缺少对应镜像
artifact 失败。完整 attempt 4 通过仅证明该 SHA 达标，不解释历史不稳定窗口的根因。

随后同步主线 `75de1168a960763be810ae320bd5709283c07f74`：保留 WebSocket 退役、
查询参数脱敏的请求日志与任务工作量标题对齐，删除孤立的 WS 观测和 usage-refresh
残留，保留 HTTP/SSE 计时、终态去重和 Grafana 链接。补充退役 upgrade 不产生代理
样本的回归。该合并改变运行时和任务 render inputs，旧 SHA 的通过卡与任务截图均
不能证明新候选就绪；任务截图已刷新、直接展示并获主人确认；仍需新的完整 Actions 和六条正式审查。
Agent VM 分配遇到 Incus agents project 实例数量上限，未取得有效租约；全量工程
验证交由 GitHub Actions，不改用本机 Docker 或共享性能环境。

主线合并候选 `9d6258f0` 的 Actions 在构建前发现锁文件不一致：主线 WS 依赖
清理同时移除了观测依赖 `metrics-util 0.20.4` 仍需要的 rand 0.9 传递依赖。
使用离线 workspace 锁定解析补回该依赖族，不更改直接依赖版本、业务行为或预算合同；
修订候选须重新完成完整 Actions，未运行的经验性场景不能记为通过。

## Current coverage

主线新增的 HTTP 流量投影回归使用扩展后的 reporter 构造合同，明确传入合成路由 `/`；
该路径归并到固定 endpoint 家族，不改变业务网络计数或测试断言。回归保留在 stateful
SQLite 资源桶的独立 `network_traffic_projection` 模块，避免继续扩张已达到行数上限的混合 harness。

`src/observability/` 提供每实例 recorder、显式 classic buckets、5 秒 CPU 与
30 秒文件/内存采样、完整 HTTP/body 生命周期、只读报告与有界浏览器接入。
资源采样节拍固定为六次 CPU 采样，持续运行不会因累计 tick 溢出提前采样。
自有 Hyper HTTP/SSE 传输、终态去重、SQLite coordinator/pool/queue/execute/ACK、
任务和投影使用实际事件。hotpath 在启动线程之前配置，最终 router 只安装一次
layer，SQLx tracing 独立于日志过滤；函数默认 10% 抽样，SQL/选定锁完整记录。
`.env` 和 `.env.local` 在同步入口统一读取，先于 hotpath/Tokio 线程启动；
全局 SQLite 仲裁器弱引用当前 recorder，拒绝绑定另一个尚活跃的 runtime，
permit 持有所属 recorder 至释放，结束的 runtime 不会被全局对象永久保留。
当前主线已退役 WebSocket 转发：upgrade 返回 501，历史业务记录保留；拒绝不产生 invocation、upstream attempt 或 stream 样本。
hotpath 关闭默认 `threads` feature，避免 SDK 每 250 ms 的 CPU/线程扫描；
进程 CPU、内存和线程数仍由应用资源采样器按锁定节拍提供，按需 CPU 调用栈使用 samply。
SDK 内部事件队列每 250 ms 排空，减少空队列的周期唤醒；事件在来源处计时，
函数抽样率、SQL/锁完整累计、15 秒抓取与报告的 2 秒期限均保持合同。
该批处理设置在启动线程前桥接；完整候选开销仍由相同负载 A/B 验收。
layer 只观测已匹配的 route template，排除任意 SPA fallback 路径与浏览器上报端点；
浏览器上报不计入应用请求或 hotpath server 指标。
浏览器 POST 路由入口先应用现有全局 120 次/分钟、客户端 30 次/分钟配额，
路由与中间件由 `src/observability/browser.rs` 统一组装，应用只合并完成的子路由。
同源、数值、JSON schema/syntax 与 body 大小拒绝也消耗配额；客户端身份仅保留在
有界限流状态，不进入指标标签。真实 HTTP 回归覆盖混合拒绝的全局配额和单客户端配额。
内部报告请求使用 hotpath 的原始 Authorization token，外部只读 API 仍使用独立
Bearer Token；函数报告的平铺分位数规范化为 API 的固定嵌套对象，回归直接使用
hotpath 依赖的序列化模型，避免手写 fixture 与真实 schema 不一致。
hotpath 固定版本的 SQL worker 使用本地有界归一化缓存；相同语句复用完全相同的
归一化结果，所有执行仍完整累计。缓存最多 256 条、原文与结果合计 2 MiB，
单条超过 64 KiB 时不缓存，元数据由条数约束；FIFO 淘汰不改变聚合指标。
SQL 导出模板同时隐藏 SQLite 双引号文本、注释和完整数值形式；双引号标识符因与
字面量无法脱离 schema 区分，也保守隐藏。真实 SQL、绑定值与执行行为不修改。
第三方来源摘要与两处补丁说明在 `vendor/`，缓存测试读取实际生产补丁文件。
源快照保留原 crate 摘要，并从同一 VCS commit 补齐公开包遗漏的 MIT 许可证。
应用 runtime 镜像在 `/usr/local/share/licenses/hotpath/` 保留该许可证。
SQL 归一化文本由整份报告的 1 MiB 上限约束，覆盖组合多个生成语句的启动触发器；
报告仍受 100 行与 2 秒限制，其他字段保留更小的类型/长度边界。
启动桥接将 hotpath 的 SQL 标签正文限制为 120 个 Unicode 字符；计入四字节
字符与 SDK 的 19 字节唯一性后缀后，仍符合 Prometheus 的 512 字节标签值边界。
该限制不截断只读 SQL 报告中的完整归一化 query。

旧 collector/writer/rollup、性能 SQLite 模块、配置和图表已移除。旧 API 仅静态 410；
任务旧 performance 摘要移除，业务主库、任务库、TerminalJournal、raw/archive 与
ModelPerformanceDetails 保留。应用提供五个 Grafana 入口与固定任务深链接。
主线任务控制与长等待验收从混合旧遥测运行器提取到
`scripts/shared-testbox-proxy-runtime-acceptance`，保留业务场景及其时间/终态断言；
旧性能 API/SQLite 容量检查完全退役。代理并发 smoke 使用新观测开关名。
旧 API 的 OPTIONS 在 CORS 外层返回同一静态 tombstone，并保留现有 CORS 策略头；
正常 API 的 preflight 行为不变。真实 HTTP server 回归覆盖三个路径的七种方法。

`ops/observability/` 提供五个固定 UID dashboard、recording rules、Grafana 告警、
私网 Compose 与凭据读取组合同，并关闭插件预安装与自动更新。项目 Skill 和 CLI 使用公网 Grafana Viewer Token；
受限 SSH 命令只 attach 绑定容器，按截止时间向一次性 profiler 容器发送 SIGINT，
并要求精确 build ID/hash 符号。采样镜像绑定不可变本地 image ID，将已核验二进制
和该实例运行库只读挂载到进程内原路径，以满足 samply live converter 的读取要求；
无网络、无 Docker socket、无宿主文件替换。镜像符号由 CI 从实际镜像提取，不重新编译采样二进制。

退役工具比较前一 major 五条完整 schema-v1 DDL，仅忽略格式和注释，拒绝额外
CHECK 和不同 DEFAULT；同时检查列类型、主键、索引与约束，核验精确文件族和路径身份。
CLI 检查归档隔离于停止容器的所有真实持久挂载。备份包含 WAL，再校验移出；
归档/恢复可重入，未知状态不移动源。CLI 还核验紧邻 v3 镜像与停止的 writer。
归档清单记录实际 schema、旧程序版本、完整性、验证/完成时间及保留期限；
不可识别或损坏状态保留失败清单和原文件，可重入的失败清单不伪装归档成功。
删除前再次检查完整成员集合，拒绝验证后新增的 WAL/SHM 或符号链接；登记成员的
身份与摘要也必须保持一致。中断后已删除的登记成员可缺失，剩余成员仍经过核验，
不会因严格集合相等检查破坏安全重入。新增成员和主文件删除后中断的回归均通过。
恢复先写入目标同目录的独占 0600 临时文件，flush/fsync 并校验 hash/integrity，
复核目标文件族为空后以原子、禁止覆盖的硬链接发布，再同步目录。普通失败仅清理
本次临时文件；进程骤停遗留的未发布临时文件保留，重试使用新文件。已发布且匹配
的完整目标可重用，缺失 alias 可补建，既有正确 alias 保留；冲突目标与 WAL/SHM
不覆盖。部分复制、发布前/alias 中断、进程骤停和竞争目标均有重入回归。

## Deployment boundary

独立运维任务拥有 101 外部平台部署与生产切换；真实 public URL、镜像 digest、
权限、容量、Authelia bypass 和 perf 条件需在上线时验证。本 PR 不合并、不发布、
不迁移生产数据。旧 SQLite 经验性卡只保留历史，不用于新候选验收。

## Validation

环境判定在每轮预热后重新准入，沿用 CPU/IO/memory PSI 阈值，以连续三次安静
样本开始测量；每轮额外等待最多 300 秒、累计 900 秒。正式窗口记录身份和 UTC/
单调起止、初末及 10 秒资源样本，超限、非法/缺失、采集错误和超过 20 秒的间隔
均为 unavailable。签发完整卡前重读六个窗口及原始证据，缺证不能通过。
artifact 白名单明确包含 `measurement-windows.json`；32 项工具和 28 项验收工具
测试通过，包括完整 DDL 变体保留、连续准入、累计等待、窗口压力/间隔、证据重读
与敏感产物拒绝。新的候选仍须通过当前 SHA 的 GitHub-hosted Actions 与六条正式审查。

已通过的迭代验证：仓库固定 Rust 1.96 的 fmt、all-targets/all-features check/clippy
及三分桶共 2952 项后端回归；Rust 1.94 宿主兼容性检查与三分桶也通过。
Web 全量单测 1745 项（6 项跳过）；
Web 类型/lint/build；18 项退役/CPU/CLI 工具回归；7 项 recording rules 和仓库合同检查。
生产与 backend-test 镜像使用仓库固定的 Rust 1.96 构建；代码修复后刷新受影响门禁。
backend-test 镜像保留 Prometheus 配置，供标签长度回归编译时读取实际抓取合同。
已对齐包含任务执行模块拆分的新主线。四张 mock UI 证据已展示、确认并落盘；
对齐主线后两个 Storybook 文件的 61 项用例通过，相关 E2E 在构建后的 mock demo 上 10 项全部通过。
77 项映射、9 项退役与 1 项合并的注册表一致性已加入自动回归。
历史 WebSocket 终态补全回归随主线转发路径退役；当前以 upgrade 拒绝不产生代理样本的独立 recorder 回归验证边界。
请求体超时夹具在实际 body 读取时启动延迟，避免 admission 期间提前入队；
启动健康检查在计时外构造无代理本地客户端，隔离宿主代理环境与客户端冷启动。
这两项定向回归已通过，包含归一化缓存补丁的完整三分桶与 Rust 静态门禁也已通过。
报告适配与标签长度修复的定向回归、静态检查和三分桶均已通过。

新候选仍须完成当前 SHA 绑定的受控 Linux Compose、HTTPS 鉴权、原实例 CPU attach
与默认观测 A/B。采样权限探针能保存 profile，不等于真实应用符号验收通过。
首轮验收因测试初始化误判 Grafana 创建接口的 201 返回值而 unavailable，
已修正就绪/创建顺序。第二轮初始化发现验收脚本未按真实 SSE 字符串参数、范围与条数
合同订阅；脚本已对齐前端请求，验证完整数据帧并持续检查 unavailable/断流。
受控流量的 CPU、代理、pool、SQL 和函数样本/分位数必须有非空有限读数，
不能仅凭查询成功接受无数据面板；未产生浏览器样本时仍保留 unknown。
速率探针中 20 req/s 出现积压，40 秒窗口约 66 秒才完成；5 req/s 的 200/200 请求
在约 40 秒完成、平均约 0.24 核。因此完整 A/B 使用 5 req/s 的非饱和流量，
同时检查完成窗口不积压，CPU/请求与 p95 的 5% 上限不变。
A/B 每个开关状态先以同一负载预热 60 秒，覆盖两个完整的资源采样周期；
速率、预热与测量时长保存在候选运行的 `run-config.json`，便于复现。
容器采样的 perf ring-buffer 需要足够 memlock 预算和 IPC_LOCK，采样容器限定为 256 MiB；
A/B 的每份合成数据副本给予应用固定 GID 写权限，避免无 capability 的应用将
宿主复制出的文件误作只读库。监控停机场景已有 50/50 请求正常完成的运行证据，
CPU 诊断探针已从原实例取得 1145 个样本并解析应用热点函数，build ID 匹配；
报告、CPU 与完整 A/B 仍须在最终候选重新验收。
报告与归一化缓存的 17 项定向回归通过；长 SQL 用例直接使用应用启动触发器的
生成语句，并经过依赖的实际归一化/序列化模型。CPU 采样识别出 SQL worker 的重复
归一化成本，因此有界缓存补丁仍须通过相同负载的完整开销复验。
正式 review 与 PR Ready 必须使用当前候选 SHA、合同与场景摘要绑定的 passed
经验性证据卡；历史或中断运行不能替代当前候选证据。完整运行日志与卡片保存在
测试机独立 run locator，生产上线条件仍由独立运维任务验证。
已运行的完整候选仍未通过开销门槛：稳定窗口曾测得 CPU/请求增加 5.54%，
后续窗口因离散程度超限失败，不能据此宣称预算改善。SDK 队列间隔探针中
工作线程 CPU 随唤醒频率下降，但后半段共享测试机同时出现 CPU、IO 与内存压力；
总开销数据不作为通过证据。批处理修复需要当前候选的完整稳定窗口重新证明。
批处理候选的 HTTPS 查询、监控故障隔离和原实例 CPU 符号解析均通过，但完整 A/B
在最后一个开启窗口发生请求超时；已完成窗口的 CPU 与 p95 离散程度也超限。
该失败运行保留日志且不作为通过证据。同步主线后的 Rust、Web、任务页 UI 和
完整经验性场景均需刷新；正式 review 与 Ready 尚未开始。

主线自动追赶同步后的 Web 验证已刷新：1745 项单测、61 项 Storybook、
8 项 demo 路由 E2E 和 2 项观测入口 E2E 通过，类型、lint、生产与 demo 构建通过。
四张当前 mock 证据已重新展示并获确认；任务页保留积压业务趋势和 Grafana 深链接。
Rust 全目标全特性 check、源码质量政策、18 项工具回归和两个主题 Spec 检查通过；
Clippy、三分桶与经验性证据以最终候选运行记录为准。

候选 `bcb88296` 的完整 Linux 验收通过：HTTPS 查询、监控停机隔离、原实例
CPU build ID/符号匹配，以及 3 对 300 秒稳定 A/B 窗口。9,000/9,000 请求完成，
CPU/请求增加 1.72%、p95 增加 2.96%。六条正式审查已完成；修复批次补齐
启动环境顺序、WS 终态、协调器绑定生命周期、所有方法的 410 和退役清单。
SQLite committed 指标已在 P1/P2 事务边界直接发出，不增加重复采集；旧 phase
字段的零是既有 missing sentinel，optional TTFT 的真实零保留为一个样本。
加强后的经验性场景使用实际浏览器批次验证 web 面板、缺测 Unknown 与 unsupported
不伪造 long-task；修复后候选必须刷新验证、经验性卡与正式审查，不能沿用旧 SHA。

候选 `5e55254d` 的 Rust 静态门禁、三分桶 2,965 项和 20 项工具回归通过。
SQLite 分桶首次出现一项既有超时夹具的 502/503 差异；三次单项和完整分桶复测通过，
没有修改断言或超时。加强的运行矩阵发现 CORS 截获 OPTIONS；精确旧/新镜像均返回
200，旧通过场景仅检查 GET/POST。监控故障隔离和原实例 CPU 符号解析通过，但 A/B
受共享 CPU 压力和磁盘耗尽中断，写入证据文件也失败；该运行为 unavailable，
不作为通过或产品开销结论。OPTIONS 修正后需刷新当前候选验证与完整运行证据。

主线项目存储总览已保留：测量主库、维护库、raw/archive 与 Xray 的实际文件范围，
删除对已移除 performance_database_path 的依赖。GNU du 集成 oracle 保留外置旧性能
文件作为排除哨兵，避免把离线退役归档重新纳入运行存储。对应状态页、demo 缺测场景
及独立 /api/system/storage 能力均保留；性能入口与任务深链接的四张已确认视觉证据
不受该状态页改动影响，最终候选仍需刷新静态、Web 与 Linux 运行验收。

性能开销验收迁移到 GitHub Actions：生产镜像构建与测量使用独立 GitHub-hosted
runner，测量串行执行同一镜像的运行时场景和三对 300 秒 A/B，保留 5% 稳定性与
开销预算。失败产物按 run/attempt 留存并阻断现有 Build Artifacts 门禁。
共享测试机只签发运行时集成卡；此前受干扰的 A/B 保留为诊断记录，不用于证明预算。
当前实现已接入工作流，是否通过以对应候选的 Actions 场景结果与验收卡为准。

候选 `c22dde3f` 的 GitHub-hosted Actions 四项经验性场景与全部 18 个 CI job 通过，
三对稳定窗口完成 9,000 个测量请求；CPU/请求增加 4.04%，p95 增加 0.24%，
两者均满足 5% 预算。该证据绑定 run `37177924143` attempt 1，只适用于该候选。
其六条正式评审完成后，一批修复补齐 CPU 采样失败的 degraded 状态，并在源头加强
SQLite SQL 模板脱敏，覆盖双引号、注释、十六进制、指数和不完整文本。
双引号标识符也保守隐藏，SQL 事件、执行次数、时间与缓存限额不变；真实查询不修改。
修复后的候选必须重新完成 Actions 验收与安全相关的六条正式评审。

主线的连续统计检查点修复保留；其 `prompt-cache-checkpoint`、旧版本读取检查和
服务代码身份记录迁入业务 `shared-testbox-proxy-runtime-acceptance`，避免重新引入
已退役的性能采集器或允许共享测试机签发观测开销预算证明。

候选 `246a6b80` 在 run `37184180828` attempt 1 完成全部四项经验性场景与 18 个 CI job，
三对稳定窗口完成 9,000 个请求，CPU/请求增加 3.83%，p95 变化 -0.17%。正式评审发现
内存读取失败的状态遗漏；同类线程、磁盘和数据库文件采样失败现在计入固定 source 的
错误计数并标记 degraded，不刷新失败来源的新鲜度、不伪造零值。缓存未就绪、平台
不支持和内存数据库文件保持缺测；正常不存在的 WAL 仍是已知零。采样周期和读取次数
不增加，业务健康和缓存/锁策略不变。修订候选必须重新通过 Actions 和正式评审。
