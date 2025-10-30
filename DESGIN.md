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

- 使用 `tokio::time::interval` 实现 10 秒轮询。
- 通过 `reqwest` 发起带认证信息的 HTTPS 请求。
- 将请求和解析逻辑封装为可单元测试的模块。
- 若接口提供分页或 cursor，仅拉取最新 20 条，并与数据库最新记录比较，避免重复插入。
- 实施指数退避策略：当出现网络或服务器错误时，将轮询间隔翻倍（最多 5 分钟），成功后恢复常规节奏。

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

## 5. 配置与可扩展性

- `Config` 结构体：管理基础 URL、轮询间隔、数据库路径和请求头。
- 未来可扩展：
  - 将数据推送到 Prometheus/Grafana。
  - 增加 Webhook 通知异常调用。
  - 引入 `tracing` 框架输出结构化日志。

## 6. 开发里程碑

1. **MVP**：完成接口抓包、配置文件读取、10 秒轮询与 SQLite 落库。
2. **稳定性**：实现错误重试、日志记录与健康检查。
3. **可视化**：提供 CLI/HTTP 接口查询最新记录，或导出 CSV。

## 7. 安全注意事项

- Cookie/Bearer Token 为敏感信息，切勿提交到版本控制。
- 若后续自动化登录，需要评估是否违反站点服务条款。
- 确保数据库文件权限仅对当前用户可读写。

按照以上设计逐步实现，即可满足主人提出的高频监控需求喵。
