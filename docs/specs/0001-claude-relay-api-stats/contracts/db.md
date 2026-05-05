# 数据库（DB）

## 统计来源维度 + 日统计快照

- 范围（Scope）: internal
- 变更（Change）: Modify
- 影响表（Affected tables）: `codex_invocations`, `stats_source_snapshots`（新）, `stats_source_deltas`（新）

### Schema delta（结构变更）

- DDL / migration snippet（草案，字段名可按实际响应调整）:

```sql
-- 1) 为既有调用记录新增来源维度
ALTER TABLE codex_invocations ADD COLUMN source TEXT NOT NULL DEFAULT 'xy';

-- 2) 外部统计快照表（模型级 + 汇总）
CREATE TABLE IF NOT EXISTS stats_source_snapshots (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  source TEXT NOT NULL,
  period TEXT NOT NULL,            -- daily (可扩展 monthly)
  stats_date TEXT NOT NULL,        -- YYYY-MM-DD（按外部口径）
  model TEXT,                      -- null 表示已汇总
  requests INTEGER NOT NULL,
  input_tokens INTEGER,
  output_tokens INTEGER,
  cache_create_tokens INTEGER,
  cache_read_tokens INTEGER,
  all_tokens INTEGER,
  cost_input REAL,
  cost_output REAL,
  cost_cache_write REAL,
  cost_cache_read REAL,
  cost_total REAL,
  raw_response TEXT,
  captured_at TEXT NOT NULL,        -- datetime('now') or external timestamp
  captured_at_epoch INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE(source, period, stats_date, model, captured_at_epoch)
);

CREATE INDEX IF NOT EXISTS idx_stats_source_snapshots_date
  ON stats_source_snapshots (source, period, stats_date, captured_at_epoch);

CREATE INDEX IF NOT EXISTS idx_codex_invocations_source_occurred_at
  ON codex_invocations (source, occurred_at);

-- 3) 外部统计增量表（按抓取时间累计）
CREATE TABLE IF NOT EXISTS stats_source_deltas (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  source TEXT NOT NULL,
  period TEXT NOT NULL,
  stats_date TEXT NOT NULL,
  captured_at TEXT NOT NULL,
  captured_at_epoch INTEGER NOT NULL,
  total_count INTEGER NOT NULL,
  success_count INTEGER NOT NULL,
  failure_count INTEGER NOT NULL,
  total_tokens INTEGER NOT NULL,
  total_cost REAL NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE(source, period, stats_date, captured_at_epoch)
);

CREATE INDEX IF NOT EXISTS idx_stats_source_deltas_epoch
  ON stats_source_deltas (source, period, captured_at_epoch);
```

- Constraints / indexes:
  - `source` 建议统一枚举值（如 `xy` / `relay`）。
  - 快照表以 `(source, period, stats_date, model, captured_at)` 唯一约束避免重复写入。

### Migration notes（迁移说明）

- 向后兼容窗口（Backward compatibility window）:
  - 现有 API 返回结构不变；新增字段默认值 `xy`。
- 发布/上线步骤（Rollout steps）:
  1. 启动时执行 `ALTER TABLE` 与新表创建。
  2. 先只写快照表，不影响现有统计。
  3. 合并统计逻辑切换后观察指标。
- 回滚策略（Rollback strategy）:
  - 保留新增列与表，切回旧统计查询即可回滚逻辑。
- 回填/数据迁移（Backfill / data migration, 如适用）:
  - 既有 `codex_invocations` 记录默认 `source='xy'`，无需回填。
