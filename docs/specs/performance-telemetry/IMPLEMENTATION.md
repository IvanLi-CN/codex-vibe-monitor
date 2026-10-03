# 外部性能观测实现

## Current coverage

`src/observability/` 提供每实例 recorder、显式 classic buckets、5 秒 CPU 与
30 秒文件/内存采样、完整 HTTP/body 生命周期、只读报告与有界浏览器接入。
资源采样节拍固定为六次 CPU 采样，持续运行不会因累计 tick 溢出提前采样。
自有 Hyper/WS 传输、终态去重、SQLite coordinator/pool/queue/execute/ACK、
任务和投影使用实际事件。hotpath 在启动线程之前配置，最终 router 只安装一次
layer，SQLx tracing 独立于日志过滤；函数默认 10% 抽样，SQL/选定锁完整记录。
`.env` 和 `.env.local` 在同步入口统一读取，先于 hotpath/Tokio 线程启动；
全局 SQLite 仲裁器弱引用当前 recorder，拒绝绑定另一个尚活跃的 runtime，
permit 持有所属 recorder 至释放，结束的 runtime 不会被全局对象永久保留。
WebSocket 准备和转发返回实际 success/error/cancelled 终态，不统一记为 unknown。
hotpath 关闭默认 `threads` feature，避免 SDK 每 250 ms 的 CPU/线程扫描；
进程 CPU、内存和线程数仍由应用资源采样器按锁定节拍提供，按需 CPU 调用栈使用 samply。
SDK 内部事件队列每 250 ms 排空，减少空队列的周期唤醒；事件在来源处计时，
函数抽样率、SQL/锁完整累计、15 秒抓取与报告的 2 秒期限均保持合同。
该批处理设置在启动线程前桥接；完整候选开销仍由相同负载 A/B 验收。
layer 只观测已匹配的 route template，排除任意 SPA fallback 路径与浏览器上报端点；
浏览器上报不计入应用请求或 hotpath server 指标。
内部报告请求使用 hotpath 的原始 Authorization token，外部只读 API 仍使用独立
Bearer Token；函数报告的平铺分位数规范化为 API 的固定嵌套对象，回归直接使用
hotpath 依赖的序列化模型，避免手写 fixture 与真实 schema 不一致。
hotpath 固定版本的 SQL worker 使用本地有界归一化缓存；相同语句复用完全相同的
归一化结果，所有执行仍完整累计。缓存最多 256 条、原文与结果合计 2 MiB，
单条超过 64 KiB 时不缓存，元数据由条数约束；FIFO 淘汰不改变聚合指标。
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

退役工具识别 schema-v1 的列类型、主键、索引与约束，核验精确文件族和路径身份；
CLI 检查归档隔离于停止容器的所有真实持久挂载。备份包含 WAL，再校验移出；
归档/恢复可重入，未知状态不移动源。CLI 还核验紧邻 v2 镜像与停止的 writer。
归档清单记录实际 schema、旧程序版本、完整性、验证/完成时间及保留期限；
不可识别或损坏状态保留失败清单和原文件，可重入的失败清单不伪装归档成功。

## Deployment boundary

独立运维任务拥有 101 外部平台部署与生产切换；真实 public URL、镜像 digest、
权限、容量、Authelia bypass 和 perf 条件需在上线时验证。本 PR 不合并、不发布、
不迁移生产数据。旧 SQLite 经验性卡只保留历史，不用于新候选验收。

## Validation

已通过的迭代验证：仓库固定 Rust 1.96 的 fmt、all-targets/all-features check/clippy
及三分桶共 2952 项后端回归；Rust 1.94 宿主兼容性检查与三分桶也通过。
Web 全量单测 1745 项（6 项跳过）；
Web 类型/lint/build；18 项退役/CPU/CLI 工具回归；7 项 recording rules 和仓库合同检查。
生产与 backend-test 镜像使用仓库固定的 Rust 1.96 构建；代码修复后刷新受影响门禁。
backend-test 镜像保留 Prometheus 配置，供标签长度回归编译时读取实际抓取合同。
已对齐包含任务执行模块拆分的新主线。四张 mock UI 证据已展示、确认并落盘；
对齐主线后两个 Storybook 文件的 61 项用例通过，相关 E2E 在构建后的 mock demo 上 10 项全部通过。
77 项映射、9 项退役与 1 项合并的注册表一致性已加入自动回归。
WebSocket 终态补全回归启用独立 recorder，验证更丰富、更少及无关的重复终态均不重计 invocation。
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
