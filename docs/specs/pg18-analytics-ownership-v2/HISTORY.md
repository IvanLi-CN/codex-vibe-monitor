# PostgreSQL Analytics Projection Ownership Cutover - History

- 2026-11-09: 根据 `docs/design/pg18-analytics-write-ownership-cutover.md` 收敛本主题。第一阶段由 PG 接管 analytics projection serving，SQLite 暂时保存业务事实和 source journal；最终业务事实迁移到 PG、SQLite 清退作为后续独立阶段。
- 2026-11-09: 将 rollback window 固定为 1 天，observation window 与 rollback window 合并为 1 天。通过 parity、restart/restore/replay、持续 serving 健康和 Journal 无缺口后才进入 cleanup gate。
- 2026-11-09: 将 Journal 与 rollback projection 的容量定义为自动策略：按最近生产数据估算完整一天的峰值并加安全余量，同时受事件数和字节数约束；容量不足时 backpressure，禁止淘汰未消费事件或伪造 coverage。
- 2026-11-09: 创建 ADR 0052 记录单一 serving owner、owner epoch、serving fence、fail-closed 和 rollback trade-off；保留 ADR 0051 作为可选 PG offload 的前置基础。
- 2026-11-11: 补充部署、观测、回滚、PG 故障恢复、容量背压、restore/replay 和 cleanup gate 运行手册；明确迁移确认后 SQLite rollback 快照保留完整 1 天，SQLite 业务事实最终清退属于独立后续 source-migration 阶段。
