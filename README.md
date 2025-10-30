# Codex Vibe Monitor

Codex Vibe Monitor 是一个用于高频采集 `https://new.xychatai.com/pastel/#/vibe-code/dashboard` 页面「最近 20 条调用记录 - Codex」数据的 Rust 项目。虽然当前版本尚未实现实时抓取功能，但仓库已经搭建好基础结构以及未来实现所需的设计文档。

- Rust 生态：计划基于 `tokio`、`reqwest` 实现 10 秒级轮询。
- 数据落库：使用 SQLite 存储接口快照，方便后续分析与可视化。
- 认证策略：通过 Chrome DevTools 导出当前登录态的 Cookie/Headers，保证接口访问可靠。

## 目录结构

- `src/`：即将实现的轮询与入库逻辑。
- `DESGIN.md`：详细的系统设计与未来开发路线。
- `README.md`：项目简介与基础使用说明（本文档）。

## 前置条件

- Rust 1.78+ 与 Cargo
- SQLite3（可选，用于手动检查数据库），推荐通过 `brew install sqlite` 安装

## 快速开始

1. 克隆或复制此目录。
2. 阅读 `DESGIN.md`，了解抓取流程、认证策略与数据模式。
3. 根据设计文档准备好 Chrome DevTools 导出的 Cookie 信息以及 `.env` 配置（尚未提交，会在后续实现）。
4. 未来实现完成后，可通过 `cargo run` 启动轮询，并查看项目根目录生成的 SQLite 数据库文件。

## 后续计划

- [ ] 解析目标页面的实际接口与请求参数。
- [ ] 将认证信息注入到 Rust 客户端请求中。
- [ ] 实现 10 秒轮询、去重逻辑与指数退避重试。
- [ ] 设计 CLI 输出与 Webhook 推送功能。

欢迎根据设计文档继续扩展项目喵。
