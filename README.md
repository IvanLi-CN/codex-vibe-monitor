# Codex Vibe Monitor

[![CI](https://github.com/IvanLi-CN/codex-vibe-monitor/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/IvanLi-CN/codex-vibe-monitor/actions/workflows/ci.yml)
[![Git Tags](https://img.shields.io/github/v/tag/IvanLi-CN/codex-vibe-monitor?sort=semver)](https://github.com/IvanLi-CN/codex-vibe-monitor/tags)
[![Container](https://img.shields.io/badge/ghcr.io%2FIvanLi--CN%2Fcodex--vibe--monitor-available-2ea44f?logo=docker)](https://github.com/IvanLi-CN/codex-vibe-monitor/pkgs/container/codex-vibe-monitor)
![Rust](https://img.shields.io/badge/Rust-2024-orange?logo=rust)
![Node](https://img.shields.io/badge/Node.js-20%2B-339933?logo=node.js&logoColor=white)
![React](https://img.shields.io/badge/React-19-61DAFB?logo=react&logoColor=black)
![Vite](https://img.shields.io/badge/Vite-7-646CFF?logo=vite&logoColor=white)
![SQLite](https://img.shields.io/badge/SQLite-3-003B57?logo=sqlite&logoColor=white)

以 10 秒固定节奏抓取「Codex 调用记录/配额快照」，写入 SQLite，并通过 REST API 与 SSE 为前端仪表盘提供实时数据流；前端使用 Vite + React 渲染图表、表格与配额状态。

## 特性

- 调度与并发：Tokio 定时器 + 信号量并发控制，60s 请求超时，智能选择连接复用或独立连接。
- 数据持久化：SQLx/SQLite，包含唯一性约束（`invoke_id` + `occurred_at`）。
- 接口与事件：Axum 提供 REST API、SSE 推送；可选托管静态 SPA。
- 前端应用：DaisyUI/Tailwind 组件化 UI，实时图表与统计概览，SSE 自动更新。
- 容器镜像：多阶段 Dockerfile，产出轻量运行时；CI 自动推送 GHCR。

## 目录结构

```
├── Cargo.toml               # Rust 包与依赖
├── src/                     # 后端：调度/HTTP API/SSE/SQLite
├── web/                     # 前端：Vite + React + TypeScript
│   ├── src/                 # 组件、hooks 与 API 封装
│   └── vite.config.ts       # 60080 端口，代理 /api 与 /events
├── Dockerfile               # 多阶段构建（前后端）
└── .github/workflows/ci.yml # CI：Lint/Test/Build/Docker 推送
```

## 快速开始（本地开发）

1. 后端

```bash
cargo run
```

默认监听 `127.0.0.1:8080`。健康检查：`GET /health`。

优雅停机：按下 `Ctrl+C` 或发送 `SIGTERM` 将触发有序关闭 —— HTTP 服务器停止接受新连接，调度器停止新一轮轮询并等待在途任务完成后退出。

2. 前端（开发模式）

```bash
cd web
npm install
npm run dev -- --host 127.0.0.1 --port 60080
```

开发服务器默认代理到 `http://127.0.0.1:8080`，也可用 `VITE_BACKEND_PROXY` 覆盖。

## 配置

在仓库根目录创建 `.env.local`（已忽略提交），常用变量如下（括号内为默认值）：

```env
XY_BASE_URL=https://new.xychatai.com
XY_VIBE_QUOTA_ENDPOINT=/frontend-api/vibe-code/quota
XY_SESSION_COOKIE_NAME=share-session
XY_SESSION_COOKIE_VALUE=<从浏览器开发者工具导出的 Cookie>
XY_DATABASE_PATH=codex_vibe_monitor.db         # (默认)
XY_POLL_INTERVAL_SECS=10                       # (10)
XY_REQUEST_TIMEOUT_SECS=60                     # (60)
XY_MAX_PARALLEL_POLLS=6                        # (6)
XY_SHARED_CONNECTION_PARALLELISM=2             # (2)
XY_HTTP_BIND=127.0.0.1:8080                    # (127.0.0.1:8080)
XY_LIST_LIMIT_MAX=200                          # (200)
XY_USER_AGENT=codex-vibe-monitor/0.1.0         # (自动)
XY_STATIC_DIR=web/dist                         # (存在时自动使用)
XY_SNAPSHOT_MIN_INTERVAL_SECS=300              # (300)
```

上述大部分变量均可使用 CLI 覆盖，例如：

```bash
cargo run -- \
  --database-path /tmp/codex.db \
  --http-bind 127.0.0.1:38080 \
  --poll-interval-secs 5
```

## HTTP API 与 SSE

- `GET /health`：健康检查，返回 `ok`。
- `GET /api/version`：返回 `{ backend, frontend }`。
- `GET /api/invocations?limit=&model=&status=`：最新记录列表（`limit` 上限由 `XY_LIST_LIMIT_MAX` 控制）。
- `GET /api/stats`：全量聚合统计。
- `GET /api/stats/summary?window=<all|current|1d|6h|30m>&limit=N`：窗口统计。
- `GET /api/stats/timeseries?range=1d&bucket=1h&settlement_hour=0`：时间序列（区间与桶宽支持 `m/h/d/mo`）。
- `GET /api/quota/latest`：最近一次配额快照。
- `GET /events`：SSE 推送，事件类型：
  - `{ type: "version", version }`
  - `{ type: "records", records: [...] }`
  - `{ type: "summary", window, summary }`
  - `{ type: "quota", snapshot }`

## Docker

构建镜像：

```bash
docker build -t codex-vibe-monitor .
```

运行（持久化数据与注入认证信息）：

```bash
docker run --rm \
  -p 8080:8080 \
  -v $(pwd)/data:/srv/app/data \
  -e XY_BASE_URL=https://new.xychatai.com \
  -e XY_SESSION_COOKIE_NAME=share-session \
  -e XY_SESSION_COOKIE_VALUE=... \
  ghcr.io/ivanli-cn/codex-vibe-monitor:latest
```

容器内默认：`XY_DATABASE_PATH=/srv/app/data/codex_vibe_monitor.db`，`XY_HTTP_BIND=0.0.0.0:8080`，`XY_STATIC_DIR=/srv/app/web`。

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

## CI / CD

- 工作流：`.github/workflows/ci.yml`（Lint/Format、后端测试、构建产物、构建并推送 Docker）。
- 版本：CI 使用脚本按 `Cargo.toml` 版本自动生成 `APP_EFFECTIVE_VERSION`，必要时自增补丁位。
- 镜像：推送至 GHCR `ghcr.io/ivanli-cn/codex-vibe-monitor`（支持 `latest`、`sha`、以及计算得出的多种版本标签）。

---

欢迎提 Issue/PR，一起把数据链路和可观测性打磨得更稳更顺！
