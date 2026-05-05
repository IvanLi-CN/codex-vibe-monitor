# SQLite 写入可靠性与后台背压（#qz42n）

## 背景 / 问题陈述

- 2026-04-26 15:30-16:30 CST 的 101 生产排查显示，新增 OAuth 上游账号期间应用层返回 44 次 `502 Bad Gateway`，同时日志出现 11 次 SQLite `database is locked`，并伴随多条 28-30 秒连接等待与 `pool timed out while waiting for an open connection`。
- 当时新增账号集中在 `2784-2791`，锁压力主要集中于 16:09-16:14 CST。慢点包括 roster、usage batch、latest sample 与后台维护写入。
- `#ay33j` 已消除 stats 读链路里的同步 rollup 写库；`#uhn89` 已把上游账号 roster 和 usage hydrate 拆分；`#jpvwj` 已引入账号分层维护。本轮处理这些能力叠加后的写锁争抢与后台任务背压。

## 目标 / 非目标

### Goals

- 建立应用层 DB pressure gate，让后台任务在 SQLite lock 或连接池拥塞后 fail-soft、退避并合并触发。
- 保护 `/v1/*` 与 OAuth callback 等前台关键路径，避免后台 rollup/backfill/retention/账号维护继续争抢连接池并放大成用户可见 502。
- 对账号维护、登录 session 清理、latest sample、account events 与维护候选查询补齐索引，降低新增 OAuth 账号 burst 时的写锁持续时间。
- 增加可观测日志：记录后台任务 skip/backoff、触发原因和任务名，压力解除后由原有 ticker / coalesced follow-up 继续收敛。
- 保持 SQLite、HTTP API、SSE 与账号池页面主要语义兼容。

### Non-goals

- 不迁移 PostgreSQL，不引入独立队列系统。
- 不重做账号池 UI、OAuth 登录页面、路由策略或 quota 口径。
- 不直接操作 101 生产部署、重启或 live cleanup。

## 范围

### In scope

- `src/db_pressure.rs`：进程级 DB pressure gate 与 pool acquire timeout / SQLite lock 识别。
- `src/maintenance/**`：hourly rollup、startup backfill、retention 的后台背压与 skip 日志。
- `src/upstream_accounts/**`：账号维护 pass 背压、维护 singleflight 保护、OAuth burst 相关索引。
- `docs/**`：部署说明、spec 与 solution 沉淀。

### Out of scope

- 前端可见 UI 改版。
- 外部 `/v1/*`、SSE、账号池 API 字段改版。
- 生产环境手工干预。

## 功能规格

### DB pressure gate

- 后台数据库工作必须先进入 `DbPressureGate::try_begin_background`。
- 同一进程默认只允许 1 个后台数据库任务同时执行。
- 任一后台任务遇到 SQLite busy/locked 或连接池 acquire timeout 时，gate 进入 30 秒 cooldown。
- cooldown 期间后台任务返回 success-like skip，并记录 task、reason 与 remaining/backoff 信息，不把错误传导给前台请求。

### Background task policy

- `hourly_rollup_refresh` 在压力下跳过，读侧继续使用现有 materialized rollup / live tail。
- `startup_backfill` 在压力下跳过当前 task，下轮 supervisor tick 再根据 progress 判断是否继续。
- `data_retention_maintenance` 在压力下跳过本轮，保留原有 interval 后续收敛。
- `upstream_account_maintenance` 在压力下跳过本轮；已有账号 actor 的 per-account dedupe 与 maintenance slots 继续负责具体账号同步的单飞与排队。

### Hot indexes

- `pool_oauth_login_sessions(status, expires_at)` 支撑 pending session 过期清理。
- `pool_upstream_account_limit_samples(account_id, captured_at DESC, id DESC)` 支撑 latest sample 查询。
- `pool_upstream_account_events(occurred_at DESC, id DESC)` 补齐全局事件时间线扫描。
- `pool_upstream_accounts(kind, enabled, status, cooldown_until, last_synced_at, last_successful_sync_at)` 支撑维护候选过滤。

## 验收标准

- Given SQLite lock 或连接池 acquire timeout，When 后台 rollup/backfill/retention/账号维护运行，Then 任务记录 skip/backoff 并返回 best-effort，不继续占用前台关键路径预算。
- Given 连续新增多个 OAuth 上游账号，When post-create sync 与维护 pass 同时存在，Then 账号 actor 继续排队/去重，后台 pass 不制造成片 30 秒连接等待。
- Given 账号池存在 180+ 账号和历史 limit samples/events，When 查询 latest sample、session cleanup、maintenance candidates，Then 查询可使用新增索引。
- Given 升级旧库，When `ensure_schema` 执行，Then 新索引以 `CREATE INDEX IF NOT EXISTS` 方式兼容创建。
- 全量门禁至少覆盖 `cargo fmt --all -- --check`、`cargo check --locked --all-targets --all-features`、相关 cargo tests、PR CI 与 review-loop。

## 质量门槛

- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `cargo test db_pressure --locked`
- `cargo test ensure_schema_adds_upstream_account_pressure_hot_path_indexes --locked`
- `cargo test maintenance_pass_dispatches_without_waiting_for_sync_completion --locked`
- `cargo test maintenance_pass_reconciles_legacy_upstream_rejected_cooldown_rows --locked`
- `cargo test sync_hourly_rollups_rebuilds_after_prompt_cache_key_backfill_updates_existing_rows --locked`
