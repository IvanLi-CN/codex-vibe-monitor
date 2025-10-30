# 系统设计概览

本项目目标是每 10 秒抓取一次 `https://new.xychatai.com/pastel/#/vibe-code/dashboard` 页面中「最近 20 条调用记录 - Codex」的数据，并将结果写入 SQLite 数据库，为后续分析或报警提供基础。

## 1. 数据来源定位

1. 使用 Chrome DevTools 打开目标页面，进入 Network 面板。
2. 在页面加载完成后，根据关键字 `vibe-code`、`codex` 或 `list` 过滤网络请求，观察最近 20 条记录所对应的 XHR/fetch 请求。
3. 记录该请求的：
   - URL（通常为 RESTful 或 GraphQL 端点）
   - HTTP 方法
   - Query 参数或请求体结构
   - 必要的 Header（尤其是 `Authorization`、`Cookie`、`x-token` 等自定义字段）
4. 确认响应 JSON 中包含的字段，并与页面显示的列（时间、ID、调用状态、消耗等）对应，写出字段映射表。

## 2. 认证与会话维持

- 账号登录后，使用 DevTools 的「Copy as cURL」功能导出请求，提取其中的 Cookie 和 Header。
- 将关键认证信息（如 Cookie、Bearer Token）保存到 `.env` 或 `config.toml` 中，项目将通过 `dotenvy` 读取。
- 设计刷新策略：
  - 若接口返回 401/403，需要提示人工重新导出 Cookie。
  - 为避免 Cookie 泄露，建议将配置文件加入 `.gitignore`，并在 `README` 中说明。

## 3. 轮询与调度策略

- 使用 `tokio::time::interval` 实现严格的 10 秒固定轮询节奏，不做指数退避，确保时间轴上不会跳过窗口。
- 单次请求设置 60 秒超时；结合 `tokio::Semaphore` 控制并行上限为 6（`timeout / interval`），避免超时堆积失控。
- `reqwest` 客户端启用 HTTP/2。当并发超过 2 条请求时，针对站点限制优先新建连接，其他情况下复用连接以减少握手开销（可通过自定义客户端池/`pool_max_idle_per_host` 实现）。
- 请求与解析逻辑拆分为独立模块，便于单元测试和后续替换。
- 若接口提供分页或 cursor，仅拉取最新 20 条，并与数据库最新记录比较，避免重复插入。

## 4. 数据入库设计

### 4.1 驱动选型

- 使用 `sqlx` 作为 SQLite 异步驱动，开启 `sqlite`、`runtime-tokio`、`macros`、`json` 等特性，确保全链路 `async/await` 风格与编译期 SQL 校验。
- `sqlx` 内部通过轻量线程池与 `libsqlite3` 交互，满足 SQLite 单写者模型下的本地嵌入式需求，不额外引入 Turso/libSQL 远端依赖。
- 后续若迁移到云端或需要复制，可再评估切换至 `libsql` 系列驱动，但当前版本以本地 SQLite 为主。

- 依赖 `sqlx`（启用 `sqlite`、`runtime-tokio`、`macros`、`json` 特性）。
- 表结构建议：

```sql
CREATE TABLE IF NOT EXISTS codex_invocations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    invoke_id TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    payload JSON,
    raw_response JSON,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(invoke_id, occurred_at)
);
```

- `payload` 用于存储提取后的关键字段，`raw_response` 保留原始 JSON 便于调试。

### 4.2 接口说明

- **请求**：`GET https://new.xychatai.com/frontend-api/vibe-code/quota`
  - 依赖 `share-session=<token>` Cookie，已保存在 `.env.local` 的 `XY_SESSION_COOKIE_VALUE`。
  - 响应体中 `data.codex.recentRecords` 即页面「最近20条调用记录 - Codex」数据；`currentUsage` 与 `subscriptions` 则可填充统计卡片。
- **字段对照**（`recentRecords`）
  - `requestTime` → 表格时间列。
  - `model` → 模型。
  - `inputTokens`、`outputTokens`、`cacheInputTokens`、`reasoningTokens`、`totalTokens` 对应数值列。
  - `cost` → 费用。
  - `status` / `errorMessage` → 成功或失败状态及错误详情。

## 5. HTTP API 与实时分发

- 在 Rust 服务中集成 Web 框架（推荐 `axum`）：
  - `GET /api/invocations`：支持分页、按状态/模型筛选。
  - `GET /api/stats`：返回调用次数、费用、Token 累计等聚合信息。
- 提供 `GET /events` SSE 端点，推送新增记录及统计增量，供前端实时订阅。
- API 与轮询在同进程运行，复用连接池：读操作使用只读事务，保持轻量。

## 6. Web SPA 界面

- 在仓库根目录新增 `web/`，使用 `Vite + Vue`（或 React）构建支持 hash 路由的单页应用。
- 界面包含两种视图：
  - **图表模式**：折线/柱状展示调用频次、费用趋势，建议选用 `ECharts` 或 `Chart.js`。
  - **列表模式**：以表格展现最新记录，包含状态标签、错误信息、搜索过滤。
- 前端初次加载通过 HTTP API 获取数据，随后订阅 SSE 实时刷新。
- 构建产物打包进 Docker 镜像，最终由 Rust 服务静态托管。

## 7. Docker 化部署

- 提供多阶段 `Dockerfile`：
  1. 编译 Rust 二进制。
  2. 构建 `web/` 前端资源。
  3. 组装精简运行镜像（例如基于 `gcr.io/distroless/cc`）。
- 通过环境变量配置 Cookie、轮询参数、数据库路径；将 SQLite 文件挂载为卷或映射到宿主。
- 可附带 `docker-compose.yml`，暴露 HTTP/SSE 端口并定义日志/数据卷策略。

## 8. 配置与可扩展性

- `Config` 结构体：管理基础 URL、轮询间隔、数据库路径、最大并行度、HTTP 客户端参数等。
- 未来可扩展：
  - 将数据推送到 Prometheus/Grafana。
  - 增加 Webhook 通知异常调用。
  - 引入 `tracing` 框架输出结构化日志。

## 9. 开发里程碑

1. **MVP**：完成接口抓包、配置读取、固定节奏轮询与 SQLite 落库。
2. **稳定性**：实现并行上限控制、超时处理、日志与健康检查。
3. **可视化**：上线 HTTP API、SSE 服务与前端 SPA（图表 + 列表）。
4. **部署**：交付 Docker 镜像与编排示例，支持生产部署。

## 10. 安全注意事项

- Cookie/Bearer Token 为敏感信息，切勿提交到版本控制。
- 若后续自动化登录，需要评估是否违反站点服务条款。
- 确保数据库文件权限仅对当前用户可读写。

按照以上设计逐步实现，即可满足主人提出的高频监控需求喵。
