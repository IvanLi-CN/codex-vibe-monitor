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

由运维安装固定版本 samply（初始锁定 `0.13.1`）、readelf、Python 3.11+ 和
`scripts/cvm-hotpath-cpu`。安装 root-owned、非 group/world writable 的固定配置
`/etc/cvm-observability/cpu.json`，内容如下（路径由运维选择，Agent 无 CLI 覆盖入口）：

```json
{
  "container": "codex-vibe-monitor",
  "profileRoot": "/home/ivan/srv/monitoring/profiles/codex-vibe-monitor",
  "symbolRoot": "/home/ivan/srv/monitoring/symbols/codex-vibe-monitor"
}
```

目录先创建，限制为专用运维身份可读写，配置路径不经过 symlink。
SSH key 配 `restrict,command="/usr/local/bin/cvm-hotpath-cpu"`；命令仅接
`capture [1..60]`，默认 30 秒/100 Hz，从绑定容器解析 PID，禁止任意 PID/shell/output。
主机 perf 权限必须实际验证，脚本不修改 sysctl 或授予通用 root shell。
固定的 samply 0.13.1 在 Linux attach 路径中不执行 duration 截止，包装命令在截止时间
向自己创建的 profiler 进程组发送 SIGINT；停止采集后最多等待 120 秒保存/符号化，
失败则终止该 profiler，应用进程不接收信号。
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
4. 启动新应用，核对新抓取、报告、任务/ACK、410 和无旧库文件访问，再切换正式流量。
5. 回滚是独立恢复操作：停止新应用，核验旧镜像/配置与业务/任务 schema 兼容，执行
   `restore --manifest <archive-manifest> --container <stopped-app> --previous-image <recorded-digest> --previous-config <recorded-config>`，再启动兼容旧镜像。工具拒绝覆盖非匹配目标。

工具回归：`PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-observability-tools.py`。
候选版本验收还必须运行 Linux 容器 attach、HTTPS 鉴权、监控停机隔离与观测开/关 A/B；
未通过这些实测不能宣布 Ready，测试机通过也不能代替 101 公网验收。

使用已提交候选运行 `scripts/shared-testbox-performance-acceptance --candidate <full SHA> --samply /srv/codex/agents/<thread>/tools/samply-x86_64-unknown-linux-gnu/samply`。
工具从该 commit 传输不可变源码、构建镜像，使用独立 Compose project、卷与私网，
不发布 host port。HTTPS 入口是受控测试 fixture；不能替代生产 Authelia 验收。
默认 20 req/s、3 次配对、每窗口 60 秒；包含 JSON/SSE 代理和固定 dashboard SSE。
运行日志、逐场景结果、A/B 原始数据与七字段 `empirical-card.json` 保存在打印出的
Agent Directory run locator，失败或环境不可用均阻止 Ready。旧 SQLite 验收卡仅作历史记录。
