# 外部性能观测实现

## Current coverage

`src/observability/` 提供每实例 recorder、显式 classic buckets、5 秒 CPU 与
30 秒文件/内存采样、完整 HTTP/body 生命周期、只读报告与有界浏览器接入。
资源采样节拍固定为六次 CPU 采样，持续运行不会因累计 tick 溢出提前采样。
自有 Hyper/WS 传输、终态去重、SQLite coordinator/pool/queue/execute/ACK、
任务和投影使用实际事件。hotpath 在启动线程之前配置，最终 router 只安装一次
layer，SQLx tracing 独立于日志过滤；函数默认 10% 抽样，SQL/选定锁完整记录。
layer 只观测已匹配的 route template，排除任意 SPA fallback 路径与浏览器上报端点；
浏览器上报不计入应用请求或 hotpath server 指标。

旧 collector/writer/rollup、性能 SQLite 模块、配置和图表已移除。旧 API 仅静态 410；
任务旧 performance 摘要移除，业务主库、任务库、TerminalJournal、raw/archive 与
ModelPerformanceDetails 保留。应用提供五个 Grafana 入口与固定任务深链接。

`ops/observability/` 提供五个固定 UID dashboard、recording rules、Grafana 告警、
私网 Compose 与凭据读取组合同，并关闭插件预安装与自动更新。项目 Skill 和 CLI 使用公网 Grafana Viewer Token；
受限 SSH 命令只 attach 绑定容器，按截止时间向 profiler 发送 SIGINT，并要求精确
build ID/hash 符号。镜像符号由 CI 从实际镜像提取，不重新编译采样二进制。

退役工具识别 schema-v1 的列类型、主键、索引与约束，核验精确文件族和路径身份；
CLI 检查归档隔离于停止容器的所有真实持久挂载。备份包含 WAL，再校验移出；
归档/恢复可重入，未知状态不移动源。CLI 还核验紧邻 v2 镜像与停止的 writer。

## Deployment boundary

独立运维任务拥有 101 外部平台部署与生产切换；真实 public URL、镜像 digest、
权限、容量、Authelia bypass 和 perf 条件需在上线时验证。本 PR 不合并、不发布、
不迁移生产数据。旧 SQLite 经验性卡只保留历史，不用于新候选验收。

## Validation

已通过的迭代验证：Rust all-targets/all-features check/clippy、三分桶共 2920 项后端
回归（模块拆分之前）；对齐主线后的 Web 全量单测 1745 项（6 项跳过）；
Web 类型/lint/build；15 项退役/CPU/CLI 工具回归；7 项 recording rules 和仓库合同检查。
已对齐包含任务执行模块拆分的新主线。四张 mock UI 证据已展示、确认并落盘；
对齐主线后两个 Storybook 文件的 61 项用例通过，相关 E2E 在构建后的 mock demo 上 10 项全部通过。
77 项映射、9 项退役与 1 项合并的注册表一致性已加入自动回归。
WebSocket 终态补全回归启用独立 recorder，验证更丰富、更少及无关的重复终态均不重计 invocation。
请求体超时夹具在实际 body 读取时启动延迟，避免 admission 期间提前入队；
启动健康检查在计时外构造无代理本地客户端，隔离宿主代理环境与客户端冷启动。
这两项定向回归已通过；当前候选仍必须通过完整三分桶与 Rust 静态门禁。

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
正式 review 与 PR Ready 必须使用当前候选 SHA、合同与场景摘要绑定的 passed
经验性证据卡；历史或中断运行不能替代当前候选证据。完整运行日志与卡片保存在
测试机独立 run locator，生产上线条件仍由独立运维任务验证。
