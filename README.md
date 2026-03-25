# Codex Vibe Monitor

[![CI Main](https://github.com/IvanLi-CN/codex-vibe-monitor/actions/workflows/ci-main.yml/badge.svg?branch=main)](https://github.com/IvanLi-CN/codex-vibe-monitor/actions/workflows/ci-main.yml)
[![CI PR](https://github.com/IvanLi-CN/codex-vibe-monitor/actions/workflows/ci-pr.yml/badge.svg)](https://github.com/IvanLi-CN/codex-vibe-monitor/actions/workflows/ci-pr.yml)
[![Git Tags](https://img.shields.io/github/v/tag/IvanLi-CN/codex-vibe-monitor?sort=semver)](https://github.com/IvanLi-CN/codex-vibe-monitor/tags)
[![Container](https://img.shields.io/badge/ghcr.io%2FIvanLi--CN%2Fcodex--vibe--monitor-available-2ea44f?logo=docker)](https://github.com/IvanLi-CN/codex-vibe-monitor/pkgs/container/codex-vibe-monitor)
![Rust](https://img.shields.io/badge/Rust-2024-orange?logo=rust)
![Bun](https://img.shields.io/badge/Bun-1.3.10%2B-f9f1e1?logo=bun&logoColor=111111)
![React](https://img.shields.io/badge/React-19-61DAFB?logo=react&logoColor=black)
![Vite](https://img.shields.io/badge/Vite-7-646CFF?logo=vite&logoColor=white)
![SQLite](https://img.shields.io/badge/SQLite-3-003B57?logo=sqlite&logoColor=white)

通过 OpenAI 兼容 `/v1/*` 代理捕获调用记录、可选轮询 CRS 日统计并写入 SQLite，再通过 REST API 与 SSE 为前端仪表盘提供实时/历史数据视图；历史 `xy` 记录与历史 quota snapshot 继续只读可查，但服务不再从 XYAI 上游抓取新数据。

## 项目截图

<img src="docs/screenshot-dashboard.png" alt="Codex Vibe Monitor Dashboard" width="1024" />

## 特性

- 调度与并发：Tokio 定时器驱动 CRS 日统计轮询，配合信号量并发控制、请求超时与连接复用策略。
- 数据持久化：SQLx/SQLite，包含唯一性约束（`invoke_id` + `occurred_at`）。
- 多源统计：支持合并外部日统计源（无明细）与本地调用记录。
- 接口与事件：Axum 提供 REST API、SSE 推送；可选托管静态 SPA。
- 前端应用：Tailwind + shadcn 风格组件化 UI，实时图表与统计概览，SSE 自动更新。
- 容器镜像：多阶段 Dockerfile，产出轻量运行时；CI 自动推送 GHCR。

## UI 文档

- 全局 UI 规范入口：`docs/ui/README.md`
- Foundations：`docs/ui/foundations.md`
- Components：`docs/ui/components.md`
- Patterns：`docs/ui/patterns.md`
- Data Visualization：`docs/ui/data-viz.md`
- Storybook：`docs/ui/storybook.md`

## 公开文档与 Storybook

- public docs 站点源码：`docs-site/`
- docs-site 默认本地地址：`http://127.0.0.1:60081`
- Storybook 默认本地地址：`http://127.0.0.1:60082`
- GitHub Pages 会把 docs-site 发布到站点根目录，并把 Storybook 装配到 `/storybook/`
- public docs 面向三类读者：想自部署的人、想继续开发的人、先想快速了解项目的人
- 当前 public docs 入口按“项目介绍 / 快速开始 / 配置与运行 / 自部署 / 排障 / 开发 / Storybook”组织

## 目录结构

```
├── Cargo.toml               # Rust 包与依赖
├── docs-site/               # Rspress public docs shell
├── src/                     # 后端：调度/HTTP API/SSE/SQLite
├── web/                     # 前端：Vite + React + TypeScript
│   ├── src/                 # 组件、hooks 与 API 封装
│   └── vite.config.ts       # 60080 端口，代理 /api 与 /events
├── Dockerfile               # 多阶段构建（前后端）
└── .github/workflows/      # CI PR / CI Main / Release / metadata gates
```

## 快速开始（本地开发）

1. 后端

```bash
cargo run
```

默认监听 `127.0.0.1:8080`。`GET /health` 现在表示 readiness：核心初始化完成并开始监听后返回 `200 ok`，否则返回 `503 starting`。历史补数会在启动后后台有界执行，不再阻塞 readiness。

优雅停机：按下 `Ctrl+C` 或发送 `SIGTERM` 将触发有序关闭 —— HTTP 服务器停止接受新连接，调度器停止新一轮轮询并等待在途任务完成后退出。

2. 前端（开发模式）

```bash
cd web
bun install
bun run dev -- --host 127.0.0.1 --port 60080
```

开发服务器默认代理到 `http://127.0.0.1:8080`，也可用 `VITE_BACKEND_PROXY` 覆盖。

3. Storybook（页面/组件复核）

```bash
cd web
bun run storybook
```

默认监听 `http://127.0.0.1:60082`。如需覆盖，可设置 `STORYBOOK_PORT`。

4. public docs（可选）

```bash
cd docs-site
bun install
bun run dev
```

默认监听 `http://127.0.0.1:60081`。如果同机已经启动 Storybook，本地 `storybook.html` 入口会跳转到当前 Storybook dev server。

前端新增 `#/account-pool/upstream-accounts` 模块，用于管理 `Codex OAuth` 与 `Codex API Key` 上游账号；页面会展示归一化后的 `5 小时` / `7 天` 窗口、最近同步状态，以及 OAuth 一次性登录会话进度。

OAuth 登录只申请官方允许的基础 scopes：`openid profile email offline_access`。OAuth 账号的 `/v1/*` 数据面现在直接内联在主服务里，不再依赖额外 sidecar，也不需要 bridge 专属配置项。

Docker / Compose 部署保持单服务即可：

```yaml
services:
  ai-codex-vibe-monitor:
    image: ghcr.io/ivanli-cn/codex-vibe-monitor:latest
    command: ["codex-vibe-monitor"]
```

## 配置

在仓库根目录创建 `.env.local`（已忽略提交），常用变量如下（括号内为默认值）：

```env
OPENAI_UPSTREAM_BASE_URL=https://api.openai.com  # (可选，默认 https://api.openai.com/)
DATABASE_PATH=codex_vibe_monitor.db              # (默认)
POLL_INTERVAL_SECS=10                            # (10；用于 CRS scheduler 基础节奏)
REQUEST_TIMEOUT_SECS=60                          # (60)
OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS=60           # (60，非 compact 上游等待超时)
OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS=180       # (180，请求体读取总超时)
OPENAI_PROXY_MAX_REQUEST_BODY_BYTES=268435456    # (256MiB)
PROXY_RAW_DIR=proxy_raw_payloads                 # (相对路径时锚定到 DATABASE_PATH 同级目录)
PROXY_RAW_MAX_BYTES=0                            # (0=unlimited, set >0 to cap)
PROXY_RAW_COMPRESSION=gzip                       # (gzip; set none to disable cold compression)
PROXY_RAW_HOT_SECS=86400                         # (24h; raw files older than this become *.bin.gz)
PROXY_ENFORCE_STREAM_INCLUDE_USAGE=true          # (true)
PROXY_USAGE_BACKFILL_ON_STARTUP=true             # (兼容保留；当前历史补数改为后台有界执行，不再阻塞 /health)
FORWARD_PROXY_ALGO=v2                            # (v2，正向代理权重算法开关: v1|v2)
MAX_PARALLEL_POLLS=6                             # (6)
SHARED_CONNECTION_PARALLELISM=2                  # (2)
HTTP_BIND=127.0.0.1:8080                         # (127.0.0.1:8080)
CORS_ALLOWED_ORIGINS=                            # (可选，逗号分隔，允许跨域 Origin 白名单)
LIST_LIMIT_MAX=200                               # (200)
USER_AGENT=codex-vibe-monitor/0.2.0              # (自动)
STATIC_DIR=web/dist                              # (存在时自动使用)
DOCS_PORT=60081                                  # (docs-site dev/preview 端口)
DOCS_BASE=/codex-vibe-monitor/                   # (GitHub Pages 项目页部署基路径)
VITE_STORYBOOK_DEV_ORIGIN=http://127.0.0.1:60082 # (docs-site 本地 Storybook 入口覆盖)
UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET=change-me      # (启用号池写入与加密落库的必填密钥)
UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID=                 # (可选，默认官方 Codex CLI client id)
UPSTREAM_ACCOUNTS_OAUTH_ISSUER=https://auth.openai.com       # (可选)
UPSTREAM_ACCOUNTS_USAGE_BASE_URL=https://chatgpt.com/backend-api  # (可选，默认 ChatGPT usage)
UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS=600       # (10 分钟)
UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS=300           # (5 分钟，账号保活 / 配额同步默认值回退)
UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS=900       # (15 分钟，提前刷新 access token)
UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS=30        # (30 天额度历史样本)
RETENTION_ENABLED=false                          # (false，需要显式开启后台保留任务)
RETENTION_DRY_RUN=false                          # (false)
RETENTION_INTERVAL_SECS=3600                     # (3600)
RETENTION_BATCH_ROWS=1000                        # (1000)
RETENTION_CATCHUP_BUDGET_SECS=300               # (300；存在 raw backlog 时，单轮 retention 连续追平预算)
ARCHIVE_DIR=archives                             # (archives，相对 DATABASE_PATH 同级目录解析)
INVOCATION_SUCCESS_FULL_DAYS=30                  # (30，上海自然日)
INVOCATION_MAX_DAYS=90                           # (90，超窗后归档并清理主库)
INVOCATION_ARCHIVE_TTL_DAYS=30                  # (30，codex_invocations archive 默认再保留 30 天)
FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS=30         # (30，上海自然日)
STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS=30         # (30，上海自然日)
QUOTA_SNAPSHOT_FULL_DAYS=30                      # (30，上海自然日)
XRAY_BINARY=xray                                 # (PATH lookup)
XRAY_RUNTIME_DIR=.codex/xray-forward             # (xray 运行时目录)

# CRS 日统计源（可选；未配置则禁用）
CRS_STATS_BASE_URL=https://claude-relay-service.nsngc.org
CRS_STATS_API_ID=<apiId>
CRS_STATS_PERIOD=daily                           # (daily)
CRS_STATS_POLL_INTERVAL_SECS=10                  # (10，默认跟随 POLL_INTERVAL_SECS)
```

价格配置已迁移到数据库持久化（可在 Web 设置页 `/settings` 在线编辑）；服务启动会自动写入默认模型价格模板。
号池维护同步配置同样支持运行期在线编辑：在 Web 的 `号池 -> 上游账号 -> 高级路由与同步设置` 中，可直接调整主频、次频与主层账号上限。`UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS` 仅作为新库或旧库缺失维护字段时的主频默认值回退；次频默认 `1800` 秒、主层上限默认 `100`。
`OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS` 仍作为后端默认回退项存在：未配置时，`/v1/responses/compact` 的“响应体首字超时”默认使用 `300` 秒；其他代理路径默认继续使用 `OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS=60` 作为非 responses 类请求的上游发送等待回退。账号池页的 `Pool routing settings` 现在只在线暴露 4 项请求链路超时：一般请求响应体首字超时、压缩请求响应体首字超时、一般请求流结束超时、压缩请求流结束超时；保存后对后续请求即时生效。
成本估算默认采用“精确模型优先 + 日期后缀模型回退”（如 `gpt-5.2-2025-12-11 -> gpt-5.2`），历史 `cost IS NULL` 的成功代理记录会在启动后由后台任务按批次增量补算（仅回填空成本，不覆盖已有值）。
raw 请求/响应文件的生命周期不再由独立环境变量控制，而是跟随 retention 窗口：新写入文件保持热数据明文 `*.bin`，超过 `PROXY_RAW_HOT_SECS=86400` 后由 retention 自动转为 `*.bin.gz`；成功调用按 `INVOCATION_SUCCESS_FULL_DAYS` 进入结构化保留，超出 `INVOCATION_MAX_DAYS` 后再归档出主库。`requestRawPath` / `responseRawPath` 应视为 opaque path，而不是假定固定后缀。
raw 冷压缩现在使用显式状态字段 `request_raw_codec` / `response_raw_codec` 跟踪 `identity | gzip`，retention 候选不再依赖 `*.gz` 后缀；当 backlog 存在时，单次 maintenance 会在 `RETENTION_CATCHUP_BUDGET_SECS` 预算内连续追平，而不是每小时只跑一小批。
`codex_invocations` archive batch 会额外写入 `archive_batch_upstream_activity` manifest，并默认按 `INVOCATION_ARCHIVE_TTL_DAYS` 在 archive 层继续保留；startup 的 upstream activity archive backfill 只读取 manifest，不再临时解压 `*.sqlite.gz`。

服务不再读取 XYAI 上游 cookie / base URL / quota endpoint；`/api/quota/latest` 仅返回数据库中已有的历史快照。

### Breaking change：公开环境变量改名

服务已停止接受 legacy `XY_*` 公共运行时键；若环境中仍保留旧键，启动会直接失败，并给出 `rename <old> to <new>` 的一对一迁移提示。

| Legacy key                                 | Canonical key                           |
| ------------------------------------------ | --------------------------------------- |
| `XY_POLL_INTERVAL_SECS`                    | `POLL_INTERVAL_SECS`                    |
| `XY_REQUEST_TIMEOUT_SECS`                  | `REQUEST_TIMEOUT_SECS`                  |
| `XY_MAX_PARALLEL_POLLS`                    | `MAX_PARALLEL_POLLS`                    |
| `XY_SHARED_CONNECTION_PARALLELISM`         | `SHARED_CONNECTION_PARALLELISM`         |
| `XY_HTTP_BIND`                             | `HTTP_BIND`                             |
| `XY_CORS_ALLOWED_ORIGINS`                  | `CORS_ALLOWED_ORIGINS`                  |
| `XY_LIST_LIMIT_MAX`                        | `LIST_LIMIT_MAX`                        |
| `XY_USER_AGENT`                            | `USER_AGENT`                            |
| `XY_STATIC_DIR`                            | `STATIC_DIR`                            |
| `XY_RETENTION_ENABLED`                     | `RETENTION_ENABLED`                     |
| `XY_RETENTION_DRY_RUN`                     | `RETENTION_DRY_RUN`                     |
| `XY_RETENTION_INTERVAL_SECS`               | `RETENTION_INTERVAL_SECS`               |
| `XY_RETENTION_BATCH_ROWS`                  | `RETENTION_BATCH_ROWS`                  |
| `XY_ARCHIVE_DIR`                           | `ARCHIVE_DIR`                           |
| `XY_INVOCATION_SUCCESS_FULL_DAYS`          | `INVOCATION_SUCCESS_FULL_DAYS`          |
| `XY_INVOCATION_MAX_DAYS`                   | `INVOCATION_MAX_DAYS`                   |
| `XY_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS` | `FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS` |
| `XY_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS` | `STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS` |
| `XY_QUOTA_SNAPSHOT_FULL_DAYS`              | `QUOTA_SNAPSHOT_FULL_DAYS`              |
| `XY_XRAY_BINARY`                           | `XRAY_BINARY`                           |
| `XY_XRAY_RUNTIME_DIR`                      | `XRAY_RUNTIME_DIR`                      |
| `XY_DATABASE_PATH`                         | `DATABASE_PATH`                         |
| `XY_FORWARD_PROXY_ALGO`                    | `FORWARD_PROXY_ALGO`                    |

上述大部分变量均可使用 CLI 覆盖，例如：

```bash
cargo run -- \
  --database-path /tmp/codex.db \
  --http-bind 127.0.0.1:38080 \
  --poll-interval-secs 5
```

## 数据分层保留与离线归档

- `invocation_rollup_hourly`、`invocation_failure_rollup_hourly`、`proxy_perf_stage_hourly`、`prompt_cache_rollup_hourly`、`upstream_sticky_key_hourly`、`forward_proxy_attempt_hourly` 是**长期在线保留**的统计层；历史查询默认优先读取这些小时桶，而不是依赖旧 raw 明细。
- `codex_invocations` 的成功记录超过 30 个上海自然日后，会先把完整行写入对应月份的离线 archive，再把主库内的原始 payload / raw response / raw file 引用精简为 `structured_only`，但保留结构化统计字段用于在线排障。
- 任意调用记录超过 90 个上海自然日后，会先归档到 `ARCHIVE_DIR/<table>/<yyyy>/<table>-<yyyy-mm>.sqlite.gz`；若 `ARCHIVE_DIR` 使用相对路径，则实际位置位于 `<DATABASE_PATH 同级目录>/<ARCHIVE_DIR 的值>/...`，写入 `archive_batches` 清单后，再从主库删除。
- `forward_proxy_attempts` 与 `stats_source_snapshots` 只保留最近 30 个上海自然日在线明细；更老数据同样执行“先归档、再清理”。
- `codex_quota_snapshots` 保留最近 30 天全量，更老日期在主库内压缩为“每个上海自然日最后一条”，被折叠掉的行进入离线归档。
- `stats_source_deltas` 长期在线保留；`/api/stats`、`GET /api/stats/summary?window=all`、`GET /api/stats/timeseries` 等长期统计路径通过 hourly rollups 保证 totals 与小时趋势在 raw retention 之后仍连续在线可查。
- 原始 payload / preview / raw file 只保证短期排障；超过在线窗口后，项目继续需要的历史能力只依赖主库统计表，不再让在线 API 回读 archive SQLite。archive 文件仅作为备份 / 导出产物；超窗 raw file 本体不保证继续可用，现有 Web UI 不提供 archived 明细在线浏览；orphan sweep 只清理超过宽限期的未引用文件。
- 运维直接扫磁盘 raw 时，统一使用镜像内置命令：`docker exec ai-codex-vibe-monitor search-raw '<needle>'`。脚本默认按容器内的 `DATABASE_PATH + PROXY_RAW_DIR` 解析搜索根目录，同时搜索 `*.bin` 与 `*.bin.gz`；加 `--regex` 可切换为正则模式，`--root` 可显式覆写路径。

首次清理建议先做 dry-run：

```bash
cargo run -- --retention-run-once --retention-dry-run
```

确认数量与 archive 路径后，再在维护窗口执行真实清理：

```bash
cargo run -- --retention-run-once
```

若要先补历史 manifest，再单独压平 raw backlog，可按下面顺序执行：

```bash
cargo run -- maintenance archive-upstream-activity-manifest --dry-run
cargo run -- maintenance archive-upstream-activity-manifest
cargo run -- maintenance materialize-historical-rollups --dry-run
cargo run -- maintenance materialize-historical-rollups
cargo run -- maintenance raw-compression --dry-run
cargo run -- maintenance raw-compression
cargo run -- maintenance prune-legacy-archive-batches --dry-run
cargo run -- maintenance prune-legacy-archive-batches
```

- 正常服务启动会在 startup backfill 后台维护里自动补齐仍缺失的 `codex_invocations` archive upstream-activity manifest，并只把主库 live tail sync 到 hourly rollups；若 `maintenance.historicalRollupBackfill.legacyArchivePending > 0`，需要显式执行 `maintenance materialize-historical-rollups` 才会把 legacy archive 物化进主库统计表。在 legacy `codex_invocations` 仍未完成物化前，`maintenance.historicalRollupBackfill.alertLevel` 会保持 `critical`，且这批 pending legacy archive 不会被 TTL 自动提前删掉。
- `maintenance prune-legacy-archive-batches` 只会删除已经完成历史物化的 backup-only archive；`codex_invocations` 仍存在 upstream activity backlog 时会保守跳过，避免过早删掉 manifest 依赖。
- `maintenance ... --dry-run` 与 `--retention-run-once --retention-dry-run` 不会再顺手执行 archive TTL 回填、manifest rebuild 或历史 rollup 物化，便于先做真正只读的容量预演。

## HTTP API 与 SSE

- 统计相关接口默认合并数据库中已有的历史 `xy`、当前 `proxy`，以及启用时的 `crs` 来源。
- `GET /health`：readiness 检查；核心初始化完成并开始监听后返回 `200 ok`，否则返回 `503 starting`。
- `GET /api/version`：返回 `{ backend, frontend }`。
- `GET /api/settings`：获取统一设置（`proxy + pricing`）。
- `PUT /api/settings/proxy`：更新 `/v1/models` 劫持与上游合并开关状态（全局持久化）。
- `PUT /api/settings/pricing`：更新价格目录（全量覆盖、全局持久化、实时生效于新请求成本估算）。
- `GET /api/invocations?limit=&model=&status=`：最新记录列表（`limit` 上限由 `LIST_LIMIT_MAX` 控制）；每条记录额外返回 `detailLevel`、`detailPrunedAt`、`detailPruneReason`，用于标记在线明细是否仍完整。超过在线窗口的明细不再承诺在线可回查。
- `GET /api/stats`：全量聚合统计；长期 totals 读取永久在线 hourly rollups，并补上尚未 sync 的 live tail。响应可选附带 `maintenance.rawCompressionBacklog`、`maintenance.startupBackfill` 与 `maintenance.historicalRollupBackfill`，用于观测 raw backlog、archive manifest backfill 与 legacy archive 物化状态。
- `GET /api/stats/summary?window=<all|current|1d|6h|30m>&limit=N`：窗口统计；`window=all` 与历史长窗口都通过 hourly rollups 保持在线连续。
- `GET /api/stats/timeseries?range=1d&bucket=1h&settlement_hour=0`：时间序列（区间与桶宽支持 `m/h/d/mo`）；跨过 raw retention 后，小时级历史查询与按天聚合都会从永久在线 hourly rollups 聚合。历史段只计算完整落在查询范围内的 archived 小时桶，不再为了首尾碎片回读 archive exact rows。
- `GET /api/stats/forward-proxy/timeseries?range=30d&bucket=1h`：forward proxy 历史小时时序，返回每个 proxy 的 request buckets 与 weight buckets。
- `GET /api/stats/perf`：代理链路阶段耗时统计（count/avg/P50/P90/P99/max）。
- `GET /api/quota/latest`：数据库中最近一次历史配额快照（服务不会再主动抓取新的 XYAI quota）。
- `ANY /v1/*`：OpenAI 兼容反向代理（请求头/请求体/状态码/响应头/响应体透明透传，包含流式响应）；`GET /v1/models` 可按设置切换为预置列表或预置+上游实时合并。
- `GET /events`：SSE 推送，事件类型：
  - `{ type: "version", version }`
  - `{ type: "records", records: [...] }`
  - `{ type: "summary", window, summary }`
  - `{ type: "quota", snapshot }`

## Docker

部署到网关/反向代理（例如 Traefik）时，请先阅读部署与安全边界说明：[`docs/deployment.md`](docs/deployment.md)。

构建镜像：

```bash
docker build -t codex-vibe-monitor .
```

运行（持久化数据；如需 CRS 或代理相关覆盖，可额外注入对应 env）：

```bash
docker run --rm \
  -p 8080:8080 \
  -v $(pwd)/data:/srv/app/data \
  ghcr.io/ivanli-cn/codex-vibe-monitor:latest
```

容器内默认：`DATABASE_PATH=/srv/app/data/codex_vibe_monitor.db`，`HTTP_BIND=0.0.0.0:8080`，`STATIC_DIR=/srv/app/web`，`PROXY_RAW_DIR=proxy_raw_payloads`（解析到 `/srv/app/data/proxy_raw_payloads`），`PROXY_RAW_COMPRESSION=gzip`，`PROXY_RAW_HOT_SECS=86400`。运行镜像已内置 `curl`、`gzip`、`search-raw` 与镜像级 `HEALTHCHECK`，会探测 `http://127.0.0.1:8080/health`。

若要在共享测试机 `codex-testbox` 上复现“真实镜像 + retention + search-raw”链路，可直接运行：

```bash
scripts/shared-testbox-raw-smoke
```

该脚本会把当前仓库同步到 `/srv/codex/workspaces/$USER/.../runs/<RUN_ID>`，在远端构建镜像并验证：

- 超过热窗口的 raw 是否从 `*.bin` 变成 `*.bin.gz`
- SQLite 中的 `request_raw_path` 是否同步更新
- `search-raw` 是否能同时命中明文与 gzip raw

若想复用已存在的远端镜像以加快验证，可加 `--reuse-image <tag>`；若希望脚本成功后自动删除本次 run 目录与镜像，可加 `--cleanup`。

推荐在 Compose 中显式覆盖 healthcheck 参数，确保启动窗口内也能正确等待 readiness：

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

Traefik 部署默认依赖 Docker health 结果决定是否把流量送到容器；如果现场 Docker provider 显式开启了 `allowEmptyServices=true`，还需要额外配置 Traefik service-level active healthcheck，对 `/health` 做兜底探测。更完整的网关示例见 [`docs/deployment.md`](docs/deployment.md)。

GHCR 发布镜像默认提供多架构 manifest（`linux/amd64` + `linux/arm64`），`stable` 会同步更新 `${image}:latest`。

## 验证与排查

- SQLite 检查：
  ```bash
  sqlite3 codex_vibe_monitor.db "SELECT invoke_id, occurred_at, status FROM codex_invocations ORDER BY occurred_at DESC LIMIT 5;"
  ```
- API 采样：
  ```bash
  curl "http://127.0.0.1:8080/api/invocations?limit=10"
  curl "http://127.0.0.1:8080/api/stats"
  curl "http://127.0.0.1:8080/api/quota/latest"
  ```
- SSE 观察：浏览器打开 `http://127.0.0.1:8080/events` 或使用 `curl`/`sse-cat`。
- 代理失败分型（近 30 分钟）：
  ```bash
  sqlite3 codex_vibe_monitor.db \
    "SELECT json_extract(payload, '$.failureKind') AS kind, COUNT(*) \
     FROM codex_invocations \
     WHERE source='proxy' AND occurred_at >= datetime('now','-30 minutes','localtime') \
     GROUP BY kind ORDER BY COUNT(*) DESC;"
  ```
  常见 kind：`request_body_read_timeout`、`request_body_stream_error_client_closed`、`failed_contact_upstream`、`upstream_handshake_timeout`、`upstream_stream_error`。

## CI / CD

- 工作流：
  - `.github/workflows/label-gate.yml`：PR 标签校验（发版意图 gate），只负责校验 `type:*` 与 `channel:*` 标签是否合法。
  - `.github/workflows/ci-pr.yml`：PR / merge_group 检查；required checks 保持不变，旧 PR run 可被新提交抢占取消。
  - `.github/workflows/ci-main.yml`：`push main` 后运行主线校验，并为当前 mainline 路径上尚未持久化的 merged commits 补齐不可变 release snapshot；发布意图直接来自各 commit 关联 PR 的当前 labels。整个 workflow 使用固定并发组 `ci-main-main` 串行执行，运行中的主线 run 不会被新 merge 取消，也不会并发推断同一个下一个版本号。
  - `.github/workflows/docs-pages.yml`：单独构建 docs-site、Storybook 与 assembled Pages artifact；PR 保留 artifact 供复核，`push main` 时部署 GitHub Pages。
  - `.github/workflows/release.yml`：由任一成功完成的 `CI Main` `workflow_run` 触发发布；也支持 `workflow_dispatch(commit_sha)` 手动 backfill。自动路径每次只发布“mainline 上最早一个尚未发布的 snapshot”，发布完成后会继续排下一个目标，从而把 burst merges 下被替换的 pending release 重新排回来；手动入口则只处理明确指定的目标 commit。整个 workflow 使用固定并发组 `release-main` 串行执行。
- PR 发版意图（labels，必须且各 1 个）：
  - `type:patch` | `type:minor` | `type:major`：触发发版（semver bump）
  - `type:docs` | `type:skip`：不发版（不推镜像/不打 tag/不建 Release）
  - `channel:stable`：稳定版
  - `channel:rc`：预发行（prerelease）
  - merged PR 的 `type:*` / `channel:*` labels 视为发布输入的一部分；合并后不要再修改这些 labels，否则后续 backfill 会按修改后的标签重建 snapshot。
- 并发约定：
  - PR checks：`cancel-in-progress: true`，新提交会取消同一 PR 的旧 run。
  - `CI Main` / `Release`：`cancel-in-progress: false`，并分别使用固定并发组 `ci-main-main` / `release-main` 做全局串行；新的 merge 不会取消较早的 main/release run。即使 GitHub 替换了 pending run，后续 `CI Main` / `Release` 也会通过 mainline catch-up 把缺失的 snapshot / release 补回来。
  - 版本号会在 `CI Main` 写 snapshot 时推断；因为 `CI Main` 已串行，所以 stable / rc 版本分配不会因为并发 merge 而撞车。
  - `workflow_run` 会接受成功完成的 `CI Main` rerun；发布幂等仍依赖 immutable snapshot、git tag 与 GitHub Release 的目标 SHA 对齐，而不是依赖 `run_attempt == 1`。
  - `workflow_dispatch(commit_sha)` 会校验目标 commit 要么已成功通过 `CI Main`，要么仅因 `Release Snapshot` 失败而阻塞；对于后者，release 会先用当前主干脚本只补齐目标 commit 的 immutable snapshot。
- 版本与 tag 规则：
  - stable：`vX.Y.Z`
  - rc：`vX.Y.Z-rc.<sha7>`（不更新 `latest`）
  - stable / rc 的版本号都会在 `CI Main` 写 snapshot 时一次性分配；手动 backfill 与 rerun 只复用 snapshot 中的既定版本，保证不会因为后续 PR label 变化、较新的 release，或部分成功 rerun 而漂移。
- 镜像：推送至 GHCR `ghcr.io/ivanli-cn/codex-vibe-monitor`
  - stable：`${image}:vX.Y.Z` 与 `${image}:latest`
  - rc：`${image}:vX.Y.Z-rc.<sha7>`（仅该 tag）
  - 发布前会分别对 `linux/amd64` 与 `linux/arm64` 做容器 smoke（`--help`、`xray version`、`/health`）；通过后再推送多架构 manifest
  - 推送后会校验版本 tag 的 manifest 必须同时包含 `linux/amd64` 与 `linux/arm64`
  - 同步创建 GitHub Release（stable 为非 prerelease，rc 为 prerelease）

## 许可证

本项目使用 MIT 协议开源，详见 `LICENSE` 文件。

---

欢迎提 Issue/PR，一起把数据链路和可观测性打磨得更稳更顺！
