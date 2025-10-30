# Codex Vibe Monitor

Codex Vibe Monitor 是一个用于高频采集 `https://new.xychatai.com/pastel/#/vibe-code/dashboard` 页面「最近 20 条调用记录 - Codex」数据的 Rust 项目。虽然当前版本尚未实现实时抓取功能，但仓库已经搭建好基础结构以及未来实现所需的设计文档。

- Rust 生态：计划基于 `tokio`、`reqwest` 实现 10 秒级轮询。
- 数据落库：使用 SQLite 存储接口快照，方便后续分析与可视化。
- 认证策略：通过 Chrome DevTools 导出当前登录态的 Cookie/Headers，保证接口访问可靠。

## 目录结构

- `src/`：当前的抓取与入库逻辑实现。
- `DESGIN.md`：详细的系统设计与未来开发路线。
- `README.md`：项目简介与基础使用说明（本文档）。

## 前置条件

- Rust 1.78+ 与 Cargo
- SQLite3（可选，用于手动检查数据库），推荐通过 `brew install sqlite` 安装

## 快速开始

1. 克隆或复制此目录。
2. 阅读 `DESGIN.md`，了解抓取流程、认证策略与数据模式。
3. 在项目根目录创建 `.env.local`（已加入 `.gitignore`），填入：
   - `XY_BASE_URL`：比如 `https://new.xychatai.com`
   - `XY_VIBE_QUOTA_ENDPOINT`：默认为 `/frontend-api/vibe-code/quota`
   - `XY_SESSION_COOKIE_NAME` 与 `XY_SESSION_COOKIE_VALUE`：来自 DevTools 的登录 Cookie，例如 `share-session=...`
   - （可选）`XY_DATABASE_PATH`：数据库文件位置，默认 `codex_vibe_monitor.db`
4. 运行 `cargo run`，程序会读取 quota 接口并将最新记录写入 SQLite。
5. 使用 `sqlite3 codex_vibe_monitor.db "SELECT * FROM codex_invocations ORDER BY occurred_at DESC LIMIT 5;"` 验证入库结果（路径按需调整）。

## 后续计划

- [x] 解析目标页面的实际接口与请求参数。
- [x] 将认证信息注入到 Rust 客户端请求中。
- [ ] 实现 10 秒轮询、去重逻辑与指数退避重试。
- [ ] 设计 CLI 输出与 Webhook 推送功能。

欢迎根据设计文档继续扩展项目喵。
