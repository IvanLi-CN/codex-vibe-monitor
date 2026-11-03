# 可选 PostgreSQL Analytics Offload 主题历史

> 这里记录主题局部生命周期、兼容性和架构背景；规范正文以 `./SPEC.md` 为准。

## Lifecycle / Compatibility

- 新增 active topic，作为长期统计 SQLite 来源与未来 PostgreSQL analytics read/offload 之间的稳定边界。
- 初始边界不改变现有 SQLite schema、business write path、TerminalJournal、Summary freshness、retention 或 raw safety。

## Replacements / Background

- PostgreSQL 连接、schema 和 read-only transaction 的校验被放在独立 runtime 中，以便 future query implementation 可以复用统一 unavailable 语义。
- schema provisioning 保留为显式运维输入；应用启动不会为了 analytics 自动执行 DDL 或业务数据迁移。
- 该主题的性能验证边界是确定性兼容与短时契约检查，不包含压力、吞吐、规模扩展或 p95 结论。

## Related Changes

- [ADR 0051: Optional PostgreSQL analytics offload](../../adr/0051-pg18-analytics-offload.md)
- [Long-term usage analytics](../5k89c-long-term-usage-analytics/SPEC.md)
- [Database retention and archive](../9aucy-db-retention-archive/SPEC.md)

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
