# 系统设计概览

本项目当前定位为：通过 OpenAI 兼容 `/v1/*` 代理捕获调用记录，可选轮询 CRS 日统计并写入 SQLite，再通过 REST API 与 SSE 为前端仪表盘提供实时与历史视图。历史 `xy` 调用记录与历史 quota snapshot 继续只读可查，但服务不再从 XYAI 上游抓取新数据。

## 1. 数据来源

### 1.1 OpenAI 兼容代理链路（主写入来源）

- 服务暴露 `ANY /v1/*`，透明转发到上游 OpenAI 兼容接口。
- 在代理链路中解析请求、响应、usage 与耗时信息，并将调用明细写入本地 SQLite。
- 新产生的在线记录以 `source='proxy'` 标记，作为当前系统的主要实时数据来源。

### 1.2 CRS 日统计源（可选）

- 通过 `CRS_STATS_BASE_URL` 与 `CRS_STATS_API_ID` 配置外部日统计来源。
- 实际采集接口为 CRS 的模型级日统计接口，本地按日期与模型做快照与增量聚合。
- 该源只提供汇总数据，不包含调用明细；写入 `stats_source_snapshots` / `stats_source_deltas`。

### 1.3 历史 XY 数据（只读兼容）

- 已存在的 `codex_invocations.source='xy'` 记录继续参与 `/api/invocations` 与 `/api/stats*` 查询。
- 已存在的 `codex_quota_snapshots` 继续通过 `/api/quota/latest` 暴露最新快照。
- 运行时不再依赖 XYAI cookie、base URL 或 quota endpoint，也不会新增新的 `xy` 调用记录或 quota snapshot。

## 2. 配置与认证

- 代理链路使用标准 OpenAI 兼容请求模型；上游地址通过 `OPENAI_UPSTREAM_BASE_URL` 控制。
- CRS 为可选能力，要求 `CRS_STATS_BASE_URL` 与 `CRS_STATS_API_ID` 成对配置。
- 数据库、HTTP 监听、并发度、超时与 retention 均通过 `.env.local` 中的通用配置项管理。
- 不再保留 XYAI 专属认证配置；部署时无需再提供历史的 XYAI cookie / quota 抓取参数。

## 3. 调度与运行策略

- 使用 `tokio::time::interval` 作为后台节拍器，但调度器只服务 CRS 统计轮询与相关汇总刷新。
- 单次请求设置超时，并结合信号量限制并发，避免外部统计源抖动导致任务堆积。
- OpenAI `/v1/*` 代理路径按请求驱动写入，不依赖后台轮询。
- 当前运行期不存在任何 XYAI legacy poll 分支、配额抓取或快照写入逻辑。

## 4. 数据持久化设计

- 使用 `sqlx + SQLite` 保存调用记录、统计快照、转发代理尝试记录与配额历史快照。
- `codex_invocations` 保留统一明细表，通过 `source` 区分历史 `xy` 与当前 `proxy` 数据。
- `stats_source_snapshots` 与 `stats_source_deltas` 用于承载 CRS 汇总快照与增量。
- `codex_quota_snapshots` 保留历史快照表，仅作为查询接口的数据来源，不再由运行时主动追加。

示意结构：

```sql
CREATE TABLE IF NOT EXISTS codex_invocations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    invoke_id TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    source TEXT NOT NULL,
    payload JSON,
    raw_response JSON,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(invoke_id, occurred_at)
);

CREATE TABLE IF NOT EXISTS stats_source_snapshots (...);
CREATE TABLE IF NOT EXISTS stats_source_deltas (...);
CREATE TABLE IF NOT EXISTS codex_quota_snapshots (...);
```

## 5. HTTP API 与实时分发

- `GET /api/invocations`：返回历史与当前调用记录，支持分页、筛选与只读兼容历史 `xy` 数据。
- `GET /api/stats`、`/api/stats/summary`、`/api/stats/timeseries`：聚合历史 `xy`、当前 `proxy` 与启用时的 `crs` 数据源。
- `GET /api/quota/latest`：读取数据库中最新的历史 quota snapshot；空库时返回 degraded default。
- `GET /events`：以 SSE 推送代理写入与统计更新，供前端实时订阅。

## 6. Web SPA 界面

- 前端位于 `web/`，使用 `Vite + React + TypeScript` 构建单页应用。
- Dashboard / Stats / Live / Settings 保持现有结构，展示调用记录、趋势图、配额卡片与代理设置。
- 页面通过 HTTP API 获取历史数据，再使用 `EventSource` 订阅 `/events` 实时刷新。
- 配额卡片展示的是数据库中已有的历史快照，而不是实时抓取 XYAI 上游结果。

## 7. 部署与扩展

- 后端与前端通过多阶段 `Dockerfile` 一体化构建，运行时静态托管 `web/dist`。
- 生产部署重点关注 SQLite 挂载、代理上游可达性、CRS 凭据与 retention 策略。
- 后续扩展应围绕 proxy 可观测性、CRS 聚合与历史查询体验展开，而不是恢复 XYAI 采集链路。
