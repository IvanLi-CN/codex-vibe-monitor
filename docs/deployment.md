# Deployment Guide

本文档说明 `codex-vibe-monitor` 在生产环境的推荐部署方式，以及设置写接口（`/api/settings/proxy` 与 `/api/settings/pricing`）的安全边界。

## Recommended Topology

- 对外只暴露网关（例如 Traefik）。
- 应用服务只在内网监听（容器网络 / 私网），不要直接暴露到公网。
- 浏览器仅访问网关域名（例如 `https://app.example.com`）。

示意：

```text
Browser -> Traefik (public 80/443) -> codex-vibe-monitor (private :8080)
```

## Security Boundary For Settings Writes

`PUT /api/settings/proxy` 与 `PUT /api/settings/pricing` 会修改全局运行配置，属于状态变更接口。

服务端会执行来源校验：

- 优先使用 `Origin` 对比请求主机信息。
- 在网关场景下，可使用网关写入的 `X-Forwarded-Host` / `X-Forwarded-Proto` / `X-Forwarded-Port` 参与同源判断。
- 对明显跨站请求（`Sec-Fetch-Site: cross-site`）拒绝写入。

这意味着以下前提必须成立：

1. 应用服务不能被外部客户端直连。
2. `X-Forwarded-*` 只应由受信任网关产生。
3. 网关与应用之间的链路是受控内网链路。

如果外部可以绕过网关直连应用端口，攻击者可伪造请求头，安全前提失效。

## Reverse Proxy Requirements

以 Traefik 为例，建议满足：

1. 仅 Traefik 暴露公网端口。
2. `codex-vibe-monitor` 不做 `ports` 直接映射到公网。
3. Traefik 路由按固定 Host 转发到应用服务。
4. 不允许旁路访问应用容器（安全组、防火墙、网络策略）。

## Readiness And Container Health

`GET /health` 现在表示 readiness，而不是“进程活着”：

- 应用完成数据库连接、schema 校验、运行时初始化并成功开始监听 `:8080` 后，返回 `200 ok`。
- 在此之前返回 `503 starting`。
- 历史补数（usage/cost/service tier/reasoning/failure classification）会在启动后后台有界执行，不再阻塞 `/health`。

推荐在 Compose 中为容器显式配置 healthcheck：

```yaml
services:
  ai-codex-vibe-monitor:
    image: ghcr.io/ivanli-cn/codex-vibe-monitor:latest
    healthcheck:
      test: ["CMD", "curl", "-fsS", "http://127.0.0.1:8080/health"]
      interval: 15s
      timeout: 5s
      retries: 6
      start_period: 60s
      start_interval: 5s
```

说明：

- 运行镜像已内置 `curl`，镜像级 `HEALTHCHECK` 也会探测相同地址；Compose 显式配置的目的是把 `interval` / `start_period` / `start_interval` 固定到部署清单里，便于 101 与 shared-testbox 一致化验证。
- Traefik 默认可直接利用 Docker health 状态，只有当容器 ready 后才把流量导入。
- 如果现场 Docker provider 显式开启了 `allowEmptyServices=true`，则必须再加一层 Traefik service-level active healthcheck，避免未 ready 实例仍被纳入路由。

Traefik 兜底示例（仅在 `allowEmptyServices=true` 时需要）：

```yaml
labels:
  - traefik.enable=true
  - traefik.http.services.codex-vibe-monitor.loadbalancer.server.port=8080
  - traefik.http.services.codex-vibe-monitor.loadbalancer.healthcheck.path=/health
  - traefik.http.services.codex-vibe-monitor.loadbalancer.healthcheck.interval=10s
  - traefik.http.services.codex-vibe-monitor.loadbalancer.healthcheck.timeout=3s
```

## Proxy Capture Runtime

以下变量均为按需覆盖；未配置时使用服务默认值：

- `DATABASE_PATH`：SQLite 主库路径；升级旧版本前请先同步新的公开 env 命名，legacy `XY_*` 公共键会在启动期直接被拒绝。
- `PROXY_RAW_DIR`：原始请求/响应落盘目录；相对路径会锚定到 `DATABASE_PATH` 同级目录，避免跟随容器工作目录漂移。
- `PROXY_RAW_MAX_BYTES`：单次请求/响应原文采集上限；默认 `0=unlimited`（支持显式配置正整数上限）。
- `PROXY_ENFORCE_STREAM_INCLUDE_USAGE`：是否在 `chat.completions` 流式请求中强制注入 `stream_options.include_usage=true`。
- `PROXY_USAGE_BACKFILL_ON_STARTUP`：兼容保留的历史补数开关说明；当前版本的历史补数已经改为后台有界执行，不再阻塞 readiness。
- `OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS`：非 compact 代理路径的上游等待超时，默认 `60` 秒。
- `OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS`：`/v1/responses/compact` 的上游等待超时，可选覆盖；默认 `180` 秒。
- `OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS`：请求体读取总超时，默认 `180` 秒；超时返回 `408`。
- `UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET`：启用号池 / 上游账号写入的必填密钥；没有它时账号列表仍可读，但新增、更新、删除与 OAuth 绑定都会被拒绝。
- `UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID` / `UPSTREAM_ACCOUNTS_OAUTH_ISSUER`：Codex OAuth 登录的 client / issuer；默认沿用官方 Codex CLI 当前参数。
- `UPSTREAM_ACCOUNTS_USAGE_BASE_URL`：OAuth 账号 usage 抓取基址；默认 `https://chatgpt.com/backend-api`，会自动拼接 `/wham/usage`。
- `UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS`：一次性 OAuth 登录会话 TTL；默认 10 分钟。
- `UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS` / `UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS`：后台账号保活与配额同步间隔 / 提前刷新窗口。
- `UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS`：上游账号 `5 小时 / 7 天` 历史样本保留天数。
- `RETENTION_ENABLED`：是否启用后台 retention/archive 维护任务，默认 `false`，上线时需要显式开启。
- `RETENTION_DRY_RUN`：全局 dry-run 开关；开启后 maintenance 只输出计划与计数，不删除数据。
- `RETENTION_INTERVAL_SECS`：常驻 maintenance 执行间隔；默认按小时调度。
- `RETENTION_BATCH_ROWS`：单批处理上限；用于降低 SQLite 长事务与锁表风险。
- `ARCHIVE_DIR`：离线 archive 根目录；相对路径会锚定到 `DATABASE_PATH` 同级目录，建议挂载到持久化卷并纳入备份。
- `INVOCATION_SUCCESS_FULL_DAYS` / `INVOCATION_MAX_DAYS`：调用明细冷热分层窗口；raw file 删除跟随这两档 retention，不再有独立的 raw retention env。
- `FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS` / `STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS`：代理尝试与统计快照的在线保留窗口。
- `QUOTA_SNAPSHOT_FULL_DAYS`：配额快照全量在线保留窗口；超窗后压缩为“每天最后一条”。

价格配置说明：

- 价格目录由 SQLite 持久化，不再依赖本地 JSON 文件路径。
- 可通过 `/settings` 页面或 `PUT /api/settings/pricing` 在线更新，并实时影响新请求的成本估算。

统计接口行为：

号池 / 上游账号运维提示：

- OAuth callback 地址固定为 `/api/pool/upstream-accounts/oauth/callback`；服务会根据实际请求 `Origin/Host` 生成 redirect URI，因此反向代理必须正确透传这两个头。
- 后台会定期刷新临近过期的 OAuth access token，并同步归一化后的 `5 小时(primary)` / `7 天(secondary)` 配额窗口；明确的授权失效会把账号转成 `needs_reauth`，但不会删除账号或清空最后成功快照。
- API Key 账号第一阶段只展示本地占位限额，不会探测真实 usage；真实计量会在后续路由阶段接入。

- `GET /api/quota/latest`：读取数据库里最近一条历史 quota snapshot；本服务不再从外部 XYAI 上游抓取新 quota。
- `GET /api/stats`、`/api/stats/summary`、`/api/stats/timeseries` 默认合并数据库中的历史 `xy`、当前 `proxy`，以及启用时的 `crs` 来源。
- `GET /api/stats/perf` 返回代理链路阶段耗时聚合统计。
- `/api/invocations` 会额外返回 `detailLevel`、`detailPrunedAt`、`detailPruneReason`，用于标记当前在线记录是否仍保留完整原始细节；`rawExpiresAt` 已移除，不再作为公开响应字段。
- `GET /api/stats` 与 `GET /api/stats/summary?window=all` 会合并在线明细与 `invocation_rollup_daily`，确保 archive/purge 后 totals 保持一致。
- 现有排障接口只查询在线 retention window，不回读离线 archive 文件。

## Header Relay Policy

应用在转发 `/v1/*` 到上游时，不会透传以下代理身份相关头：

- `Forwarded`
- `Via`
- `X-Real-IP`
- `X-Forwarded-For`
- `X-Forwarded-Host`
- `X-Forwarded-Proto`
- `X-Forwarded-Port`
- `X-Forwarded-Client-Cert`

因此：

- 上游不会通过这些头识别当前服务的代理层信息。
- 下游（浏览器）也不会从应用响应里看到这些头被回传。

## Verification Checklist

部署后至少检查：

1. 外部无法直接访问应用监听端口（例如 `:8080`）。
2. 容器启动时，`curl -fsS http://127.0.0.1:8080/health` 会先返回 `503 starting`，待监听完成后切到 `200 ok`。
3. 通过网关域名访问应用页面和 API 正常，且重启窗口内不会被提前转发到未 ready 实例。
4. 正常同源写入（页面设置保存）返回成功。
5. 非同源 `Origin` 请求写入返回 `403`。

## Runtime Troubleshooting（代理断流）

遇到 Codex 端 `stream disconnected before completion` / `error decoding response body` 时，优先按以下口径排查：

1. **看失败分型（30 分钟窗口）**
   - `request_body_read_timeout`：客户端上传过慢或代理前置链路阻塞（对应 `408`）。
   - `request_body_stream_error_client_closed`：客户端在上传阶段断开（对应 `400`）。
   - `failed_contact_upstream`：代理到上游连接失败（对应 `502`）。
   - `upstream_handshake_timeout`：上游握手超时（对应 `502`）。
   - `upstream_stream_error`：上游流式响应中途失败（通常表现为下游读流报错）。

2. **确认上游地址是否走内网**
   - Docker Compose 推荐优先使用同网络服务名（例如 `http://ai-claude-relay-service:3000/openai`），避免公网握手放大抖动。

3. **核对超时口径**
   - 默认值：`OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS=60`（非 compact 上游等待超时）、`OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS=180`（`/v1/responses/compact` 上游等待超时）、`OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS=180`（请求体读取总超时）。
   - 若部署中显式覆盖这些环境变量，需要确认三者职责没有被混用。

## Retention And Archive Operations

- 首次 backlog cleanup 先执行 `cargo run -- --retention-run-once --retention-dry-run`，确认预计归档行数、目标 archive 路径与磁盘变化。
- 正式清理使用 `cargo run -- --retention-run-once`；执行顺序必须保持 `导出成功 -> archive_batches manifest 成功 -> 删除源数据`。
- archive 文件按上海自然月切分；若 `ARCHIVE_DIR` 为相对路径，则实际目录形如 `<DATABASE_PATH 同级目录>/<ARCHIVE_DIR 的值>/<table>/<yyyy>/<table>-<yyyy-mm>.sqlite.gz`（例如 `archives/...`）。
- `codex_invocations` 成功记录超过 30 个上海自然日后，会先把完整行写入离线 archive，再在主库内精简为 `structured_only`；任意调用超过 90 天后清理主库明细。
- `forward_proxy_attempts`、`stats_source_snapshots` 只保留近 30 天在线明细；`codex_quota_snapshots` 近 30 天逐条保留，更老日期压缩为每天最后一条。
- 原始 payload / preview / raw file 只保证短期排障；长期依赖离线 archive 中的 SQLite 归档行，超窗 raw file 本体不保证继续可用，而不是在线 UI。orphan sweep 只会清理超过宽限期的未引用文件，以避免误删进行中的请求落盘文件。
- 常驻 maintenance 只做 `wal_checkpoint(PASSIVE)` 与 `PRAGMA optimize`；首次真实 cleanup 完成后，再在维护窗口人工执行一次 `VACUUM`。
