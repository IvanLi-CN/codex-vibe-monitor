# 外部性能观测

应用只在内存累计指标，Prometheus 抓取并保留历史，Grafana 提供图表和告警。
本目录是部署合同；101 的域名、镜像 digest、卷容量、认证与 perf 权限由上线任务验证。

## 应用配置

```dotenv
OBSERVABILITY_ENABLED=true
METRICS_BIND=0.0.0.0:9091
METRICS_TOKEN_FILE=/run/secrets/metrics-token
OBSERVABILITY_READ_TOKEN_FILE=/run/secrets/observability-read-token
GRAFANA_PUBLIC_URL=https://grafana.example.com
```

两个 Token 必须不同，secret 文件只挂载给所需服务。应用与监控 Compose
连接共享 `cvm-monitoring` 网络，保留应用原业务/出站网络。应用服务的网络 alias
为 `codex-vibe-monitor`，9091/6772/6770 不发布 host port。6770 固定 loopback。
Grafana 不参与业务请求成功条件，exporter 故障只令观测 degraded。

## 平台部署

先准备外部监控网络、Token 文件与 Grafana admin 密码文件；设置
`METRICS_TOKEN_FILE`、`GRAFANA_ADMIN_PASSWORD_FILE`、`GRAFANA_PUBLIC_URL`、
`OBSERVABILITY_SECRET_GID`，
再执行 `docker compose -f compose.yml -p cvm-monitoring up -d`。
Token 与密码文件放在运维身份拥有的私密目录（0700），文件为 0640，group 是
`OBSERVABILITY_SECRET_GID` 指定的专用读取组。Compose 只为两个监控容器追加此组，
保留各自默认 UID/GID；应用容器也须追加同组以读取其挂载的两个 Token。
只挂载各服务必要的单个文件，不挂载整个 secret 目录。不要用 0600 的运维用户
文件直接挂给非 root 容器，也不要把文件改成全员可读。
Compose 关闭插件预安装和自动更新，避免后台下载安装不需要的插件；所需功能来自固定镜像与 provisioning。
Grafana 仅绑定 host loopback，由现有入口反向代理到公网 HTTPS；Prometheus 没有 host port。
上线固定已验证镜像 digest（Compose 的版本 tag 是待验证的初始版本）。

抓取间隔 15 秒，默认保留 30 天 / 8 GB，先满足者触发清理。为 WAL、head 和
compaction 留额外磁盘，8 GB 并非严格卷容量上限。依照实际负载确定容量。
固定数据源 UID 是 `cvm-prometheus`；五个 dashboard UID 为
`cvm-overview/proxy/sqlite/runtime/web`（均以 `cvm-` 开头）。
应用 classic Histogram 使用 `_bucket`，hotpath job 仅 native Histogram。
分位数与样本数、新鲜度一同判断；缺测显示 unknown，不补成业务零。
Grafana 告警通知 contact point 由平台配置，本目录不配置发送目标。

## 公网只读机器访问

人类入口继续使用 Authelia。为项目创建独立 Grafana organization 与 Viewer
service account，隔离其他数据源；OSS Viewer 不等于企业版逐数据源 RBAC。
入口保留 `Authorization`，仅对以下必要机器路径跳过交互登录，Grafana 继续校验 Token：

- `GET /api/datasources/uid/cvm-prometheus`
- `GET /api/datasources/proxy/uid/cvm-prometheus/api/v1/query` 与 `query_range`
- `GET /api/dashboards/uid/cvm-{overview,proxy,sqlite,runtime,web}`

使用 gcx 时按锁定版本实际请求追加必要只读资源/查询路径，POST 仅开放真正查询入口。
不要整站 bypass。验收有效 Token 返回 JSON、无效/缺失 Token 拒绝、写入拒绝，
并确认人类登录保护有效。Agent 查询无需 SSH 或 Prometheus 公网地址。

项目 CLI：`scripts/cvm-observe <signal> [--minutes 30]`，Token 来自私有文件。
官方 gcx 使用相同 URL、organization、Viewer Token 和 datasource UID。
curl+jq 可通过固定 proxy 路径查询；将 Bearer header 放在权限 0600 的 curl config，
使用 `curl --config <private-config> --fail --get --data-urlencode 'query=up{job="cvm-app"}' <Grafana-HTTPS>/api/datasources/proxy/uid/cvm-prometheus/api/v1/query | jq`。
应用报告使用另一个 Token，CLI 的 `server/sql/functions` 分支只访问固定白名单。
Agent 排查顺序见[项目 Skill](../../.agents/skills/performance-investigation/SKILL.md)。

## CPU 采样与符号

生产 release 构建保留优化及 line-table debug 信息，不为采样重启应用。
`Dockerfile` 的 `observability-symbols` target 导出 build ID 目录、精确二进制与 revision/hash manifest。
主线 candidate 与手动发布构建从实际 smoke 镜像提取同一二进制，分别上传
`cvm-symbols-<full SHA>-amd64/arm64` CI artifact（保留 90 天）。运维下载匹配发布
revision/platform 的产物，核对 `image-identity.json` 与运行镜像 digest，再装入 symbolRoot。
也可从指定 image digest 用 `docker create` / `docker cp` 取出
`/usr/local/bin/codex-vibe-monitor`，再执行 `scripts/export-observability-symbols.py`。
运行二进制与符号 hash/build ID 必须相同；不要使用单独重新编译的 profiling binary。

由运维从上游 release 下载本机平台的 samply `0.13.1` 并校验发布的 SHA256，
将二进制命名为 `samply`，与 `ops/observability/cpu/Dockerfile` 放在独立构建目录。
执行 `docker build -t cvm-cpu-profiler:0.13.1 <构建目录>`，用
`docker image inspect --format '{{.Id}}' cvm-cpu-profiler:0.13.1` 取得不可变本地 image ID。
这是按需启动并删除的采样容器，不增加常驻服务；镜像不挂 Docker socket、不联网，
只接收固定原实例 PID、已核验二进制、该实例只读运行库与项目 profile 目录。
运行库从原实例实际 maps 的固定系统库目录复制，最多 32 个/32 MiB，逐文件核验
校验和后只读挂载，完成即删除；中断遗留目录会阻止后续采样，交由运维检查。
samply live converter 需要读取进程内映射路径，`--symbol-dir` 不会补齐这个路径；
包装命令将精确二进制只读挂载到原 `/usr/local/bin/codex-vibe-monitor`，不改宿主文件。
主机安装 Docker、readelf、Python 3.11+ 和 `scripts/cvm-hotpath-cpu`，受限命令由
具备这些操作权限的专用运维身份执行。安装 root-owned、非 group/world writable 的固定配置
`/etc/cvm-observability/cpu.json`，内容如下（路径由运维选择，Agent 无 CLI 覆盖入口）：

```json
{
  "container": "codex-vibe-monitor",
  "profileRoot": "/home/ivan/srv/monitoring/profiles/codex-vibe-monitor",
  "symbolRoot": "/home/ivan/srv/monitoring/symbols/codex-vibe-monitor",
  "profilerImage": "sha256:<docker image inspect 返回的 64 位十六进制 image ID>"
}
```

目录先创建，限制为专用运维身份可读写，配置路径不经过 symlink。
SSH key 配 `restrict,command="/usr/local/bin/cvm-hotpath-cpu"`；命令仅接
`capture [1..60]`，默认 30 秒/100 Hz，从绑定容器解析 PID，禁止任意 PID/shell/output。
主机 perf 权限必须实际验证，脚本不修改 sysctl 或授予通用 root shell。
还需验证专用采样身份的 memlock 预算能覆盖运行实例各线程的 perf ring-buffer；
权限探针能 attach 单线程进程不证明真实应用可用。测试机 profiler 使用有界
256 MiB memlock limit 和隔离的 `PERFMON`、`IPC_LOCK` 能力，生产按原实例 attach
结果核定，不修改全局 perf 配置；应用容器不添加这些能力。
不对采样容器设置 CPU quota：固定版本以可用核数枚举 perf CPU，配额可能使它只
监听 CPU 0 而漏掉原实例线程。使用低 CPU shares、512 MiB 内存与 64 个 PID 上限，
并核验 profile 有实际样本及正确应用 build ID；空 profile 不记为成功。
固定的 samply 0.13.1 在 Linux attach 路径中不执行 duration 截止，包装命令在截止时间
向自己创建的唯一 profiler 容器发送 SIGINT；停止采集后最多等待 120 秒保存/符号化，
包装命令的截止/失败信号仅发给该 profiler。samply attach 自身会在初始化 perf
事件时短暂停止并恢复目标线程；上线前需核验原实例 attach 对业务延迟的影响。
同实例用 flock 单并发。采样保留 7 天、总量 512 MiB，先清理已验证到期产物，
不足时拒绝采样；中断 pending 文件计入容量，由运维检查后处理。
manifest 含 UTC、容器/PID、revision/build ID、频率及 profile/符号侧文件校验和。
采样使用固定版本的 `--unstable-presymbolicate`，profile 和同名 `.json.syms.json` 一起
计入容量与到期清理。通过受控 SSH 文件读取取回这两个指定产物，`samply load` 使用
成熟查看器；不建公网下载站。

## 旧系统退役与回滚

这是 major breaking change。先移除两个 `PERFORMANCE_*` env，旧 API 仅返回 410，
task detail 的 `performance` 字段已移除。新应用不创建、读取或写入旧性能库，
业务主库、任务库、TerminalJournal、raw/archive 和调用阶段数据保留。

1. 固定旧镜像 digest 与旧配置文件，备份业务/任务数据；停止旧应用并确认 writer 已排空。
2. 明确 `--source`（含自定义路径/alias）、`--business-db`、`--data-root` 与挂载外
   `--archive-root`，执行 `scripts/retire-performance-db.py archive --container <stopped-app> --previous-image <image@sha256:digest> --previous-config <old-config> --source <exact-db> --business-db <business-db> --data-root <mount> --archive-root <outside-mount> --operation-id <id>`。
3. 工具先核验归档不在停止容器的任何持久化挂载下，再核验旧镜像 OCI version 属于紧邻的 v2 major；归档时停止的 writer 容器 image ID
   必须匹配指定 digest。未知版本和直接跨 major 均拒绝，不移动源文件。
   再核验 schema v1/marker/文件身份，SQLite backup 含已提交 WAL，验证 integrity/hash
   后移出精确旧文件族；manifest 阶段为 identified → verified → archived。未知/损坏/失败保持源。
   中断复用同 operation id 前向续作，归档保留 90 天，不导入 Prometheus、不双写。
   清单保存验证后的 schema DDL、旧程序版本、完整性结果与 verified/archived 时间，
   `retainUntil` 从归档完成起计算 90 天；`cutoverAt` 的 scope 是性能文件族移出，
   应用及正式流量切换仍须完成下一步验收。未知 schema 或损坏记录为 `failed`，
   保存可读文件族的身份/hash并原地保留；同 operation id 重入返回同一失败。
   确认并修复失败原因后使用新的 operation id，不能覆盖原失败清单。
4. 启动新应用，核对新抓取、报告、任务/ACK、410 和无旧库文件访问，再切换正式流量。
5. 回滚是独立恢复操作：停止新应用，核验旧镜像/配置与业务/任务 schema 兼容，执行
   `restore --manifest <archive-manifest> --container <stopped-app> --previous-image <recorded-digest> --previous-config <recorded-config>`，再启动兼容旧镜像。工具拒绝覆盖非匹配目标。
   恢复数据先在目标同目录的 0600 临时文件完成同步与校验，再原子发布且禁止覆盖。
   普通复制/校验失败不会留下部分最终目标；中断后可重复相同 restore 命令，完整匹配
   目标会复用并补齐缺失 alias，正确的既有 alias 保留。冲突目标或 WAL/SHM 原地保留并拒绝。
   进程骤停可能留下 `.目标文件名.restore-*` 私有未发布文件；重试不使用或删除这些文件。
   如需清理，停止所有恢复进程后逐个核验确切路径和归属，仅处理已确认的遗留文件，禁止宽泛清理。

工具回归：`PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-observability-tools.py`。
候选版本验收还必须运行 Linux 容器 attach、HTTPS 鉴权、监控停机隔离与观测开/关 A/B；
完整性能验收由 CI PR 的 GitHub-hosted Actions job 执行，未通过不能宣布 Ready。
任何测试环境通过都不能代替 101 公网验收。

使用已提交候选运行 `scripts/shared-testbox-performance-acceptance --candidate <full SHA> --samply /srv/codex/agents/<thread>/tools/samply-x86_64-unknown-linux-gnu/samply --seconds 300 --rate 5`。
工具从该 commit 传输不可变源码、构建镜像，使用独立 Compose project、卷与私网，
不发布 host port。HTTPS 入口是受控测试 fixture；不能替代生产 Authelia 验收。
共享测试机入口只验证 JSON/SSE 代理、固定 dashboard SSE、monitoring 停机隔离与
原容器 CPU attach，写出 `runtime-card.json`；它不能替代完整性能验收卡，也不执行 A/B。

CI PR 的 `Observability Performance Image` 使用当前提交构建生产镜像，并传输镜像
ID、revision 与 archive checksum；`Observability Performance Budget` 在另一个
`ubuntu-24.04` runner 上加载同一镜像，先完成运行时场景，再串行测量 A/B。
测量期间没有编译或并行测试套件；采样工具固定 samply 0.13.1 与 checksum。
默认 5 req/s、3 次交替配对、每窗口 300 秒与 60 秒预热；保留两组 CV ≤5% 的稳定性
门槛、CPU 非饱和与窗口末尾无积压检查，以及 CPU/完成请求和 p95 增幅 ≤5% 的预算。
资源准入不满足时标记 unavailable；窗口不稳定时不输出已接受的增幅结论。

资源准入使用 PSI `some avg10/avg60`：CPU <2%、IO <5%、memory <0.1%。每轮
预热后按 20 秒间隔取得连续三次安静样本；额外等待每轮最多 300 秒、总计 900 秒。
正式窗口记录开关、配对编号、UTC/单调起止与初末压力，随后每 10 秒采样；压力
超限、样本缺失/非法、采集错误或间隔超过 20 秒都标记 unavailable 并阻断门禁。
等待超限不自动重跑；70 分钟 job 上限、5% CV 和开销预算保持不变。
`measurement-windows.json` 保存逐窗口判定，`resource-observer.jsonl` 与
`environment-admission.jsonl` 保存带窗口身份的原始证据，失败时也按白名单上传。

应用浏览器上报使用固定 60 秒限流窗口；客户端身份由现有反向代理负责。
101 上线验收必须确认入口覆盖客户端传入的 `X-Real-IP`，且应用监听端口不能
从公网直接访问；该检查不能用测试 fixture 推定通过。

每次 attempt 的 `observability-acceptance-<run_id>-<attempt>` artifact 保留 14 天，
包含原始样本、比较摘要、runner/资源环境、场景结果与七字段 `empirical-card.json`。
只上传明确白名单，不上传 Token、数据库或私有 Compose 配置；卡片 locator 指向 Actions
run/attempt。失败或环境不可用阻断现有 `Build Artifacts` 必需检查，无需新增远端保护规则。
旧共享测试机 A/B 与旧 SQLite 验收卡仅作历史诊断记录。
