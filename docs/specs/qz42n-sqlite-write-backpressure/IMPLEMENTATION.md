# SQLite 写入可靠性与后台背压实现记录

## 实现状态

- [x] 新增进程级 `DbPressureGate`，识别 SQLite busy/locked 与 pool acquire timeout。
- [x] 后台 hourly rollup、startup backfill、retention 与 upstream account maintenance 接入 pressure gate。
- [x] 后台任务在 pressure cooldown 或已有后台 DB 任务执行中 fail-soft skip，并记录结构化 warning。
- [x] 补齐 OAuth login session、upstream account events、latest sample 与 maintenance candidate 索引。
- [x] 保持 `/v1/*`、SSE、账号池 HTTP API 与 UI 语义不变。

## 关键实现点

- `src/db_pressure.rs` 提供全局 gate，避免改动 `AppState` 结构并减少测试构造面影响。
- `try_begin_background` 默认 singleflight 后台 DB 工作；这不是全局数据库锁，只是把低优先级维护任务从连接池竞争里移开。
- `record_error` 会把 SQLite lock 与 pool acquire timeout 转换为 30 秒 cooldown；压力解除后原有维护 ticker / follow-up 会自然重试。
- `upstream_account_maintenance` 仍保留账号 actor 的 per-account dedupe、maintenance slots 与 post-create 立即同步语义；本轮只限制维护 pass 本身。
- 新索引全部采用 `CREATE INDEX IF NOT EXISTS`，不新增表字段，不需要数据迁移。

## 验证记录

- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `cargo test db_pressure --locked`
- `cargo test ensure_schema_adds_upstream_account_pressure_hot_path_indexes --locked`
- `cargo test maintenance_pass_dispatches_without_waiting_for_sync_completion --locked`
- `cargo test maintenance_pass_reconciles_legacy_upstream_rejected_cooldown_rows --locked`
- `cargo test sync_hourly_rollups_rebuilds_after_prompt_cache_key_backfill_updates_existing_rows --locked`
- `cargo test run_upstream_account_maintenance_once --locked` 与 `cargo test upstream_account_maintenance --locked` 当前未匹配到测试，仅作为过滤确认，不计入覆盖证据。

## Migrated Task-Ticket Sections

## 文档更新

- `docs/specs/README.md`
- `docs/specs/qz42n-sqlite-write-backpressure/IMPLEMENTATION.md`
- `docs/specs/qz42n-sqlite-write-backpressure/HISTORY.md`
- `docs/solutions/performance/sqlite-write-pressure-backpressure.md`
- `docs/deployment.md`
- `README.md`
