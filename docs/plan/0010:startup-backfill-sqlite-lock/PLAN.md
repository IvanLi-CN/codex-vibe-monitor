# 启动回填 SQLite 锁冲突修复

## Goal

修复升级到 `v0.6.0` 后启动阶段回填逻辑触发 `database is locked` 并导致服务重启抖动的问题，同时统一项目默认 SQLite 并发参数，保证不同环境行为一致。

## In / Out

### In

- 启动连接阶段显式设置 SQLite `journal_mode=WAL` 与 `busy_timeout=30s`。
- 重构 `backfill_proxy_usage_tokens`：避免“流式读 + 循环写同表”导致的锁竞争。
- 为启动回填新增“仅锁冲突可重试”的包装器（固定 2 次尝试，重试间隔 3 秒）。
- 重试后仍失败时保持启动失败退出（不降级吞错）。
- 补充锁冲突重试与回填批处理路径测试。

### Out

- 不改动 HTTP API 路径与响应 schema。
- 不改动数据库 schema。
- 不新增可配置开关（默认行为由代码强制）。
- 不修改与本问题无关的统计筛选逻辑或前端展示。

## Acceptance Criteria

1. Given 服务启动时存在可回填记录，When 执行启动回填，Then 不再因回填自身锁竞争触发连续重启。
2. Given 启动回填遇到 `SQLITE_BUSY/SQLITE_LOCKED`，When 首次失败，Then 仅触发一次延迟重试并输出可观测日志。
3. Given 重试后仍为锁冲突，When 启动流程结束，Then 服务按既定策略失败退出。
4. Given 回填成功，When 检查 `proxy + success` 记录，Then `total_tokens` 回填保持幂等且不重复污染。
5. Given 任意新环境首次启动，When 建库并连接，Then SQLite 使用 `WAL` 且 `busy_timeout` 为 30 秒。

## Testing

- `cargo fmt --check`
- `cargo test`
- `cargo check`
- 覆盖：
  - 回填批处理幂等路径
  - 锁冲突重试成功路径
  - 锁冲突重试失败路径
  - SQLite 连接参数默认值（WAL + busy timeout）

## Risks

- 回填改为分页批处理后，单批大小过大会增加单次事务时长（默认 200，需要在回归中确认）。
- 强制 WAL 依赖目标环境文件系统支持共享内存与 WAL 文件写入。
- busy timeout 提高会延后真实锁异常暴露时间，需要通过日志区分“短暂竞争”与“持续异常”。

## Milestones

- [ ] M1 启动连接参数显式化（WAL + busy timeout）
- [ ] M2 回填算法重构为批处理读写
- [ ] M3 启动回填锁冲突重试包装器
- [ ] M4 测试补齐与回归验证
