# Codex Vibe Monitor

Codex Vibe Monitor 负责以 10 秒固定节奏抓取 `https://new.xychatai.com/pastel/#/vibe-code/dashboard` 中「最近 20 条调用记录 - Codex」接口的数据，存入 SQLite，并通过 HTTP API 与 SSE 为前端仪表盘提供实时更新。

- **轮询调度**：`tokio` 定时器 + 信号量控制并发，60 秒超时，上限 6 条并发请求，自动选择 HTTP/2 连接复用或独立连接。
- **数据持久化**：`sqlx` + SQLite，保留原始响应及常用字段，确保唯一性约束 (`invoke_id` + `occurred_at`)。
- **服务接口**：`axum` 提供 REST API（列表/统计）、SSE 实时推送以及静态 SPA 托管。
- **前端界面**：`web/` 目录内的 Vite + React + TypeScript + TailwindCSS + DaisyUI 单页应用，支持图表/列表双视图，默认通过 SSE 实时刷新。
- **容器化**：多阶段 Dockerfile 同时构建 Rust 二进制与前端静态资源，产出轻量运行镜像。

## 目录结构

```
├── Cargo.toml
├── Dockerfile               # 生产镜像构建脚本
├── DESGIN.md                # 系统设计与补充约束
├── src/                     # Rust 后端（轮询、API、SSE、静态资源）
├── web/                     # 前端 SPA（Vite + React + DaisyUI）
│   ├── src/
│   ├── package.json
│   └── vite.config.ts
└── codex_vibe_monitor.db    # 默认数据库（已在 .gitignore 中）
```

## 运行前准备

1. **环境依赖**
   - Rust 1.78+（建议使用 `rustup`）
   - Node.js 20+（Vite 开发与构建）
   - SQLite（可选，用于手动检查数据）

2. **配置认证信息**
   在项目根目录创建 `.env.local`（已加入 `.gitignore`）并填入：

   ```env
   XY_BASE_URL=https://new.xychatai.com
   XY_VIBE_QUOTA_ENDPOINT=/frontend-api/vibe-code/quota
   XY_SESSION_COOKIE_NAME=share-session
   XY_SESSION_COOKIE_VALUE=<通过 DevTools 导出的 Cookie>
   # 以下可按需覆盖默认值
   # XY_DATABASE_PATH=codex_vibe_monitor.db
   # XY_POLL_INTERVAL_SECS=10
   # XY_REQUEST_TIMEOUT_SECS=60
   # XY_MAX_PARALLEL_POLLS=6
   # XY_HTTP_BIND=127.0.0.1:8080
   # XY_STATIC_DIR=web/dist
   ```

   `XY_STATIC_DIR` 默认会尝试使用 `web/dist`，若不存在则仅提供 API/SSE。

## 本地开发流程

### 1. 后端服务

```bash
cargo run
```

启动后端后将会：

- 每 10 秒拉取最新 quota 数据并写入 SQLite
- 在 `XY_HTTP_BIND` 指定端口开放：
  - `GET /api/invocations?limit=50&model=&status=` 最新记录
  - `GET /api/stats` 聚合统计
  - `GET /events` SSE 实时推送 `{ type: "records", records: [...] }`
  - `GET /health` 健康检查
  - 静态 SPA（若配置了 `XY_STATIC_DIR`）

### 2. 前端（开发模式）

```bash
cd web
npm install
npm run dev
```

Vite 会读取 `VITE_BACKEND_PROXY`（默认 `http://localhost:8080`）并自动代理 `/api` 与 `/events` 请求，保证本地开发时前端可直接调用后端服务。

### 3. 打包前端

```bash
cd web
npm run build
```

构建产物位于 `web/dist`，后端默认会在静态目录存在时自动托管这些文件。

## Docker 部署

构建镜像：

```bash
docker build -t codex-vibe-monitor .
```

运行容器（映射数据目录与环境变量）：

```bash
docker run --rm \
  -p 8080:8080 \
  -v $(pwd)/data:/srv/app/data \
  -e XY_BASE_URL=https://new.xychatai.com \
  -e XY_SESSION_COOKIE_NAME=share-session \
  -e XY_SESSION_COOKIE_VALUE=... \
  codex-vibe-monitor
```

容器镜像默认环境：

- 数据库路径 `/srv/app/data/codex_vibe_monitor.db`（可通过挂载卷持久化）
- 静态资源目录 `/srv/app/web`
- 对外端口 8080

## 调试与验证

- 查询最新记录：

  ```bash
  sqlite3 codex_vibe_monitor.db 'SELECT invoke_id, occurred_at, status FROM codex_invocations ORDER BY occurred_at DESC LIMIT 5;'
  ```

- 使用 `curl` 验证 API：

  ```bash
  curl "http://127.0.0.1:8080/api/invocations?limit=10"
  curl "http://127.0.0.1:8080/api/stats"
  ```

- SSE 测试（浏览器或 CLI 工具如 `hey`、`curl`、`sse-cat`）。

## 参考文档

- `DESGIN.md`：包含轮询策略、SSE、前端结构与 Docker 要求等详细设计。
- `src/main.rs`：主程序，调度、HTTP 服务入口与数据库 schema 管理。
- `web/src/`：React 组件、hooks、API 封装等实现细节。

有新的需求或补充要求时，请同步更新设计文档和 README，确保开发与部署体验持续一致喵。
