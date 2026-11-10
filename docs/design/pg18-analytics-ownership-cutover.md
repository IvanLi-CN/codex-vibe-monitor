# PG18 Analytics Ownership Cutover Runbook

本手册把 [PG18 analytics ownership 设计](./pg18-analytics-write-ownership-cutover.md) 落成可执行的部署、观测、回滚和清理步骤。它只覆盖 analytics 派生 projection 的 ownership；SQLite 业务事实和 source journal 在当前阶段仍是权威来源。

## 1. 部署前置条件

部署必须先确认已合并 `projection-runtime` 基线和 `ownership-test-matrix` 的 closed 验收证据。没有完整 target mapping、连续 Journal coverage、统一 generation/revision、digest/parity 或 rollback capacity 证据时，不得启用 owner。

使用同一业务容器和现有 PG read/replay 配置，只改变独立开关：

```sh
docker compose --env-file deploy/analytics-ownership-v2-disabled.env up -d --wait
docker compose --env-file deploy/analytics-ownership-v2-enabled.env up -d --wait
```

`ANALYTICS_BACKEND` 保持原值；`ANALYTICS_OWNERSHIP_V2` 默认 `disabled`，且只在容器重启时读取。启动必须读取 durable owner state：状态与开关不一致时先 fence 并返回 `AnalyticsUnavailable`，不得启动双 writer 或隐式 fallback。

启用前记录 `J0`、source/archive manifest、schema/hash/bounds/row count、当前 owner epoch、generation、serving ack、retention floor 和自动容量估算。容量配置同时观察事件数和字节数，估算覆盖完整一天生产峰值并包含安全余量。

## 2. Handoff 和观测窗口

1. `disabled` 重启后保持 SQLite serving，验证 PG schema、projection target、applied ledger、checkpoint 和 rollback 表可写。
2. `enabled` 重启后启动 shadow，连续 replay `[J0, Jc]`，在同一 PG transaction 提交 projection DML、applied ledger 和 checkpoint。
3. 只有 facts、keys、daily/hourly、interval/read-model、repair/bucket/publication、checkpoint 全部 parity，且 owner CAS 写入同一 epoch/generation/revision/fence 后，才允许 PG serving。
4. CAS 成功后，所有 analytics 读取使用一个 `REPEATABLE READ, READ ONLY` PG snapshot；SQLite rollback projection 继续从同一 Journal 更新。
5. 观测窗口固定为 1 天，与 rollback window 合并。窗口内必须完成至少一次重启、restore/replay、持续 serving 健康、Journal 无缺口、容量和备份证据。任一证据缺失，继续保留 rollback projection，不进入 cleanup gate。

每 5 分钟记录一次 serving fence、PG replay lag、Journal oldest unconsumed age、checkpoint 连续性、rollback coverage、容量使用率、backpressure 状态、`AnalyticsUnavailable` 次数和 restore/replay 结果。容量达到自动安全阈值告警；达到硬上限前必须 backpressure。

## 3. 回滚和 PG 故障

### 配置回滚

1. 将 `ANALYTICS_OWNERSHIP_V2=disabled` 写入同一容器的 env file，保留 `ANALYTICS_BACKEND`。
2. 重启并立即 fence PG owner；handoff 未完成或状态不一致时保持 unavailable。
3. 校验一天内的 SQLite rollback projection 与最新 serving fence、generation、retention floor 一致。
4. proof 完整则 CAS 回 SQLite serving；proof 不完整则从 `J0`/source archive 重建 SQLite projection，Journal 不跳过、不截断。
5. 回滚或 rebuild 完成前禁止提供 partial、mixed-generation 或 fabricated-zero analytics 响应。

### PostgreSQL 故障

PG 连接、schema、checkpoint、generation、parity 或 snapshot 失败时，停止 PG serving、保留 Journal 和 rollback projection，并返回稳定的 `AnalyticsUnavailable`。不得把故障自动转成重型 SQLite 查询。恢复 PG 后从最后已提交 checkpoint 重放，重复事件依靠 `(projection, change_id)` 幂等；发现 gap 或 retention gap 时新建 generation 并重新 bootstrap。

## 4. 容量背压和恢复重放

Journal 与 rollback projection 以最近生产峰值的一天为基线，分别受事件数和字节数上限约束。接近安全阈值时告警；达到硬上限前暂停低优先级 source 写入或按既有业务优先级延期，并保持 analytics unavailable。禁止淘汰未被 PG 消费的事件、跳过 replay、推进虚假 coverage 或删除 rollback projection。

恢复顺序固定为：读取最后连续 checkpoint → 校验 source identity/generation/schema/digest → 按连续区间 replay → 在同一事务写入 projection、applied ledger、checkpoint → 校验 parity → 更新 serving fence。restore/replay 可重复执行，任何中断从最后提交 checkpoint 继续。

## 5. Cleanup gate 和后续边界

迁移确认后，SQLite rollback 快照和本阶段不再 serving 的派生表至少保留完整 1 天。只有以下证据全部存在才允许删除：

- handoff parity、owner CAS 和每个 serving watermark 的追溯证据；
- 一天 observation/rollback window 已完成；
- 至少一次 restart、restore/replay 成功且 Journal 无缺口；
- rollback projection coverage、事件数/字节数容量和备份证据达标；
- 当前 owner、generation、retention floor 和删除目标经过人工确认。

cleanup 只删除不再 serving 的 SQLite 派生表、rollback 快照和旧 projection state。业务事实、source journal、原始 payload、archive source、archive maintenance metadata 不得由本闸门删除。删除动作必须可审计、可重入；任一检查失败则保留快照并继续告警。

最终完全清退 SQLite 属于后续独立 source-migration 阶段：另行定义业务事实和 source journal 的 PG schema、双写/切换、事实 retention、恢复和兼容性合同。当前 cleanup gate 不授予删除 SQLite 业务事实的权限，也不代表 SQLite 已完成最终退役。

## 6. 收口证据

本手册的文档合同由 `.github/scripts/check_pg18_analytics_ownership_docs.py` 校验。实现交付还必须引用 `ownership-test-matrix` closed 证据、`projection-runtime` 合并 SHA、运行命令、观测窗口摘要、故障恢复结果和 cleanup gate 决策；没有这些证据只能停留在 shadow/unavailable。
