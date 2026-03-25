# Permanent online hourly stats retention（#h9r2m）

## 状态

- Status: 已实现
- Created: 2026-03-21
- Updated: 2026-03-25

## 背景 / 问题陈述

- 现有 retention 会把旧的 `codex_invocations` 与 `forward_proxy_attempts` 明细归档出主库。
- 旧实现依赖 `invocation_rollup_daily` 或直接扫 live 明细，导致历史统计只能保住 totals，保不住“永久在线可查询的小时桶”。
- 统计页、错误分布、失败摘要、性能统计、prompt cache conversation、sticky key conversation，以及 forward proxy 请求时序，都需要在 raw 明细过期后继续在线读取小时级聚合。
- CRS 已不存在，本规范不再为 CRS 保留任何兼容层或验收要求。

## 目标

- 将在线长期统计层从“daily fallback + live raw scan”升级为“permanent hourly rollup”。
- 为 invocation 主统计、失败分类、代理性能阶段、prompt cache conversation、sticky key conversation、forward proxy attempt 提供永久在线小时桶。
- 让 `/api/stats/timeseries` 在跨过 raw retention 后仍支持 `1h/6h/12h/1d` 查询，不再强制降级为 `1d`。
- 保持 `/api/stats/summary`、`/api/stats/errors`、`/api/stats/failures/summary`、`/api/stats/perf`、prompt cache conversations、sticky key conversations 现有响应结构不变，仅替换底层查询来源。
- 新增 `/api/stats/forward-proxy/timeseries`，提供 ranged hourly request buckets 与 weight buckets。
- 明确在线历史真相源只允许来自主库统计表；archive SQLite 仅保留为 backup/export，不再参与在线 API、startup catch-up 或常规 retention replay。

## 非目标

- 不保证原始 invocation / error 明细永久在线保留。
- 不为 `/api/stats/errors/others` 等逐行钻取接口补做无限期在线历史。
- 不在本次变更中移除 legacy `invocation_rollup_daily`，该表仅保留给回滚与兼容迁移观察使用。
- 不恢复或扩展 CRS 相关 schema、接口或 UI。

## 方案

### 永久小时桶表

- `invocation_rollup_hourly`
- `invocation_failure_rollup_hourly`
- `proxy_perf_stage_hourly`
- `prompt_cache_rollup_hourly`
- `upstream_sticky_key_hourly`
- `forward_proxy_attempt_hourly`

### 写入路径

- 每次写入 `codex_invocations` 时，同事务 upsert：
  - invocation totals hourly rollup
  - invocation failure hourly rollup
  - proxy perf stage hourly rollup
  - prompt cache hourly rollup
  - sticky key hourly rollup
- 每次写入 `forward_proxy_attempts` 时，同事务 upsert `forward_proxy_attempt_hourly`。
- 所有小时桶统一按 UTC `bucket_start_epoch` 对齐整点。

### 启动补齐与归档连续性

- 启动时先创建新表，再执行 live-table replay。
- `hourly_rollup_live_progress` 记录 live replay 到的最新 row id，避免重复累计。
- `hourly_rollup_materialized_buckets(target, bucket_start_epoch, source)` 记录历史小时桶是否已被物化进主库统计表，供 retention 删除旧明细前校验完整性。
- `archive_batches.historical_rollups_materialized_at` 记录 legacy archive 是否已经完成一次性历史回灌。
- retention 在删除 live raw rows 前必须先同步 live tables 到 hourly rollups。
- 正常启动不再临时解压 archive SQLite 做 replay；若检测到 legacy archive 尚未物化，只暴露 maintenance backlog 并等待显式 maintenance。

### 查询层

- `/api/stats/summary` 的 `window=all` 读取 `invocation_rollup_hourly`，只补尚未 sync 的 live tail。
- `/api/stats/timeseries` 改为 rollup-first：历史窗口直接读取 hourly rollups，再按请求 bucket 重新聚合；超过在线保留窗后不再为了补首尾碎片回读 archive exact rows。
- `/api/stats/errors` 与 `/api/stats/failures/summary` 对超出 raw retention 的范围读取 `invocation_failure_rollup_hourly`。
- `/api/stats/perf` 对超出 raw retention 的范围读取 `proxy_perf_stage_hourly`，使用 mergeable histogram 近似 `p50/p90/p99`。
- prompt cache 与 sticky key 的 aggregate totals 改为读取对应 hourly rollups；最近 24h request trace 仍读取 raw rows。
- `/api/stats/forward-proxy` 的 24h 请求桶改为读取 `forward_proxy_attempt_hourly`。
- `/api/stats/forward-proxy/timeseries` 提供历史 hourly request buckets 与 weight buckets；当前仅支持 `bucket=1h`。
- `/api/invocations/:id/pool-attempts` 只读主库 live rows；超出在线保留窗时返回空结果，不再对 archive 做 fallback。

### legacy archive 维护

- 新增 `maintenance materialize-historical-rollups [--dry-run]`，一次性把 legacy archive 中项目仍需要的历史统计回灌到主库统计表，并写入桶完整性元数据。
- 新增 `maintenance prune-legacy-archive-batches [--dry-run]`，只删除已经完成历史物化的 backup-only archive batch。
- 缺失 archive 文件不应再导致 `/api/stats*` 返回 `500`；历史统计查询的正确性只依赖主库统计表。

## 数据约束

- counts、tokens、cost、avg、max 必须保持精确。
- first-byte 与 proxy perf percentile 允许通过固定桶直方图近似计算。
- 小时桶表不参与 retention 删除。
- retired proxy 即便不在当前 runtime 中，只要历史 rollup 仍在范围内，也应通过历史接口继续可见。

## 验收标准

- 旧 invocation / forward proxy attempt raw rows 被归档删除后，对应小时级统计查询结果不变。
- `/api/stats/timeseries?range=<older-than-retention>&bucket=1h` 返回连续小时桶，且不再返回 `bucketLimitedToDaily=true`。
- `/api/stats/summary?window=all` 不再依赖 `invocation_rollup_daily` 作为在线统计主来源。
- `/api/stats/errors`、`/api/stats/failures/summary`、`/api/stats/perf` 在 archive 边界前后保持分类与总量连续。
- prompt cache conversations 与 sticky key conversations 在 raw 明细归档后仍保留历史 totals / first seen / last activity。
- `/api/stats/forward-proxy/timeseries` 能返回历史 request buckets 与 weight buckets，且 `forward_proxy_attempts` 被清理后结果仍连续。
- 缺失 legacy archive 文件时，`/api/stats*` 与 `/api/stats/summary` 继续可用，不再因为在线请求触发 archive exact read 而报 `500`。

## 验证

- `cargo check`
- Rust targeted tests covering:
  - invocation hourly continuity across archive boundary
  - forward proxy historical hourly continuity after retention
  - prompt cache / sticky key aggregate continuity
- Rust targeted tests covering:
  - legacy archive materialization + prune
  - missing archive file no longer breaks historical stats
  - pool attempt route is live-only
- `cd web && bun run test -- api.test.ts`
