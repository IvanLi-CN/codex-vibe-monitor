# PostgreSQL Analytics Projection Ownership Cutover - Implementation

## Current status

本主题已合并 ownership 状态、Journal fence、PG replay 状态字段和长期统计路由等基础结构，并已接入受 parity gate 保护的 owner CAS、PG daily/hourly shadow/serving 和 rollback 镜像；完整 ownership cutover 尚未实现，不能把合并记录或任务 closed 状态作为可上线启用的证据。interval/read-model、repair/bucket/publication 仍缺少完整维护与读取链路，SQLite rollback projection 仍未形成独立的连续 Journal consumer。

`ANALYTICS_OWNERSHIP_V2=enabled` 目前只允许进入 shadow/preparation 阶段，不能作为生产 owner 切换完成的信号；若 PG projection readiness、parity、rollback 和剩余 target gate 未全部满足，服务保持 unavailable 或 SQLite shadow serving。`ANALYTICS_BACKEND` 仍保持现有部署值。既有 `ANALYTICS_BACKEND=postgres` 的 facts/key read replica 与 projection ownership 是两个合同，前者不应要求 source 已成为 PG owner。

本地修复候选恢复 disabled 模式的 PG read/replay，补全 source snapshot 字段，在就绪后持续消费 Journal，并让重启沿用已有 baseline/checkpoint。后续切片已增加 PG daily/hourly rollup、projection state/checkpoint、SQLite shadow 快照物化、行数与 SHA-256 校验，以及 fenced PG overview/series 读取；启用仍受完整 owner parity、interval/read-model、连续 rollback 和 cleanup gate 约束。迁移按不可变的已部署 V1 和增量 V2/V3/V4 执行；识别已发布的错误 V1 checksum 变体但不重写其历史。候选仍须通过真实 PG fixture、审核和交付后才能用于生产。

本地修复证据见 [runtime repair validation](assets/runtime-repair-validation.json)、[persistent-state record](assets/runtime-repair-state-record.json) 和 [version impact](assets/runtime-repair-version-impact.json)。这些证据只覆盖修复候选，不替代完整 V2 验收。

## 需求与证据状态

| 需求                  | 实现状态                                                              | 仍需完成的证据                                               |
| --------------------- | --------------------------------------------------------------------- | ------------------------------------------------------------ |
| REQ-PG18-OWN-001、010 | 阶段边界文档已定义                                                    | 后续 source migration 独立交付                               |
| REQ-PG18-OWN-002      | 开关解析与重启捕获已实现；自动切换未实现                              | enabled handoff、disabled rollback 的生产入口回归            |
| REQ-PG18-OWN-003、004 | owner/fence、applied/checkpoint 和 daily/hourly shadow 基础结构存在   | interval/read-model target、统一 generation 和完整 owner CAS |
| REQ-PG18-OWN-005      | facts/key 与 daily/hourly 支持 fenced PG snapshot；其余长期统计未完成 | 所有长期统计 endpoint 单一 PG snapshot                       |
| REQ-PG18-OWN-006      | rollback 表、copy helper 和 serving 时镜像路径存在                    | 独立 rollback Journal consumer、可提升证明和 outage rebuild  |
| REQ-PG18-OWN-007、008 | 容量和 gate 基础结构存在                                              | 一天容量实测、完整 target parity、备份和恢复证据             |
| REQ-PG18-OWN-009      | 不完整 enabled 被拒绝；其余失败路径需实测                             | 真实 handoff 中断、PG outage、rollback rebuild               |
| REQ-PG18-OWN-011      | 手册已落盘                                                            | 完整一天实际观测与执行证据                                   |

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

`ownership-test-matrix` 的 closed 记录不能证明这些端到端场景通过；其遗留测试文件未进入主干。`projection-runtime` 的合并只证明代码已集成，不证明完整 ownership 生命周期可用。cleanup gate 还必须记录完整一天 observation/rollback window、restart/restore/replay、容量/备份证据和删除目标确认。迁移确认后 SQLite rollback 快照至少保留 1 天，业务事实和 source journal 不得由本阶段清理。
