# PostgreSQL Analytics Projection Ownership Cutover - Implementation

## Current status

本主题已完成需求澄清和设计落盘，尚未修改运行时代码、数据库迁移或部署配置。实现必须先以本目录 `SPEC.md`、[ADR 0052](../../adr/0052-pg18-analytics-write-ownership.md) 和 [ownership cutover design](../../design/pg18-analytics-write-ownership-cutover.md) 为合同。

## Planned implementation coverage

- `src/config.rs` 增加独立的 `ANALYTICS_OWNERSHIP_V2` 解析和脱敏配置，保持 `ANALYTICS_BACKEND` 的既有行为。
- SQLite schema 增加 owner state、owner epoch、serving fence、rollback projection identity、capacity observation 和 cleanup gate；结构变更、当前数据初始化和历史回放必须分开、可观测、可暂停。
- `src/analytics_changes.rs` 保持 source transaction 和 Journal identity，增加连续 generation、retention floor、自动容量和 backpressure admission；未消费事件不得在 PG lag 时被淘汰。
- `src/pg_analytics/` 增加完整 projection target、applied ledger、checkpoint、owner CAS 和单事务 serving fence；reader 不能混读 SQLite。
- `src/long_term_stats/` 将 daily/hourly、interval、repair、bucket 和 publication maintenance 改为按 owner fence 路由，并在 PG owner 期间继续维护 SQLite rollback projection。
- handoff、rollback、restore/replay 和 cleanup 使用同一 durable lifecycle；rollback window 和 observation window 均为 1 天。

## Rollout sequence

1. `disabled` 启动保持既有 SQLite serving，校验 PG schema、source coverage、目标映射和 rollback capacity prerequisites。
2. `enabled` 重启启动 shadow、archive/snapshot replay、Journal catch-up、identity/digest parity 和 projection parity；未完成时保持 SQLite serving 或 unavailable，不能提前 claim owner。
3. 在连续 `Jc`、统一 generation/revision、完整 target parity 和 owner epoch CAS 成功后切换 serving fence；切换后所有 analytics read 使用 PG 单一快照。
4. 进入一整天的 observation/rollback window，验证 serving lag、restore/replay、restart、capacity 和 backup evidence。
5. cleanup gate 通过后删除不再 serving 的 SQLite 派生表、rollback projection 和旧 projection state；业务事实、source journal、archive source 和 archive metadata 保留。
6. 未来业务事实迁移到 PG 和 SQLite 完全清退另立 source-migration 交付，不复用本主题 cleanup gate。

## Validation contract

- `bun --no-install skills/bin/spec_contract_check.ts --path docs/specs/pg18-analytics-ownership-v2/SPEC.md`
- 目标 Rust 回归覆盖 owner state、Journal gap、generation fencing、capacity/backpressure、PG applied ledger、single-snapshot read、rollback rebuild 和 cleanup gate。
- 使用仓库规定的 backend resource profiles 执行 SQLite Journal、archive file IO 和 PG fixture 验证；不通过手工全套 Cargo 命令替代 profile runner。
- Docker 合同使用同一现有 PG read/replay 配置，只切换 `ANALYTICS_OWNERSHIP_V2`，并验证两次重启、handoff 中断、PG 故障、rollback rebuild、restore/replay 和最终 cleanup gate。
- 业务事实未来迁移前，必须另外完成 source-migration Spec、ADR、persistent-state migration record 和兼容性验证。

## Evidence boundary

本主题没有 UI surface 变化，不生成 visual evidence。运行时代码、数据库迁移、部署重启和远程环境验证需要在后续实现交付中单独记录。

## Runbook and closeout contract

部署、观测、回滚、PG 故障恢复、容量背压、restore/replay 和 cleanup gate 的执行顺序见 [ownership cutover runbook](../../design/pg18-analytics-ownership-cutover.md)。运行手册要求 `ANALYTICS_OWNERSHIP_V2` 与现有 `ANALYTICS_BACKEND` 解耦、同一容器重启生效，并在 owner state 不一致时 fail closed。

`ownership-test-matrix` closed 证据覆盖重启交接、PG 不可用、重复 replay、回滚重建、容量 warning/backpressure、无数据丢失和恢复后 checkpoint 连续性；`projection-runtime` 合并基线覆盖 serving fence 和 projection routing。cleanup gate 还必须记录完整一天 observation/rollback window、restart/restore/replay、容量/备份证据和删除目标确认。迁移确认后 SQLite rollback 快照至少保留 1 天，业务事实和 source journal 不得由本阶段清理。
