# Daily timeseries archive continuity and subday bucket guard（#nm7ep）

## 状态

- Status: 已完成（4/4）
- Created: 2026-03-11
- Last: 2026-03-19

## 背景 / 问题陈述

- `RETENTION_ENABLED=true` 与 `INVOCATION_MAX_DAYS=7` 的线上 retention 会把较老 proxy invocation 明细归档出主库，仅保留 `invocation_rollup_daily` 的按天汇总。
- 2026-03-11 的热修已经恢复 `bucket=1d` 的历史连续性，但 `最近 1 个月 + 每 12 小时` 一类子日级请求仍只读取在线 `codex_invocations` 与 `stats_source_deltas`，导致归档窗口中段整段空窗。
- `/api/stats` 与 `summary?window=all` 已经会合并 `invocation_rollup_daily`，所以当前故障表现为“总量还在，但按日图表历史变空”。
- 需要继续以热修方式把统计页改成“归档窗口只提供按天粒度”，避免对子日级 bucket 伪造精度或继续展示误导性的空窗。

## 目标 / 非目标

### Goals

- 让 `bucket=1d` 的 `/api/stats/timeseries` 在 invocation 明细被 archive 后，继续从 `invocation_rollup_daily` 读取对应历史日桶。
- 保证纯 rollup 日桶只回填 count/tokens/cost，不伪造 first-byte latency 统计。
- 当请求范围跨过 `INVOCATION_MAX_DAYS` 的 live 子日级窗口时，自动把 `1m/5m/15m/30m/1h/6h/12h` 请求提升为 `1d`，并在响应中返回实际生效粒度与可用粒度列表。
- 统计页只展示当前范围真正受支持的 bucket 选项；当旧状态残留无效 bucket 时，自动回退到 `1d`。
- 增加后端回归测试，覆盖 archived rollup day、live + rollup continuity、proxy-only source scope。
- 增加前端回归测试，覆盖归档窗口下的 bucket 选项过滤与 stale bucket 回退。

### Non-goals

- 不恢复在线 `codex_invocations` 明细，不回灌 archive sqlite.gz。
- 不恢复在线 `codex_invocations` 明细，不把 daily rollup 均摊为伪造的 `12h/1h` 细粒度。
- 不在本次热修里补做任意时区的 archived subday 重分桶。
- 不在本 PR 内直接改线上部署或要求生产重启。

## 范围（Scope）

### In scope

- `src/stats/mod.rs`：新增按日期范围读取 `invocation_rollup_daily` 的 helper，并复用 `InvocationSourceScope` 过滤。
- `src/api/mod.rs`：继续改造 `fetch_timeseries` / `fetch_timeseries_daily`，在子日级请求跨过 archive 边界时强制切换到 `1d`，并返回 `effectiveBucket`、`availableBuckets`、`bucketLimitedToDaily` 元信息。
- `src/tests/mod.rs`：新增/调整 Rust 测试覆盖 archive continuity、archive-aware subday fallback 与 live-window subday continuity。
- `web/src/lib/api.ts`、`web/src/lib/statsBuckets.ts`、`web/src/pages/Stats.tsx`：限制统计页可选 bucket，并在后端强制切桶后自动同步到 `1d`。
- `web/src/lib/statsBuckets.test.ts`：覆盖归档窗口 bucket 过滤与 stale bucket 回退。
- `docs/specs/README.md`：登记本热修 spec 并在 PR 收敛阶段同步状态。

### Out of scope

- 数据迁移、schema 变更。
- 非统计页的图表/组件 bucket 交互改造。
- 生产明细恢复脚本。

## 接口契约（Interfaces & Contracts）

- `GET /api/stats/timeseries` 保持现有 query 不变；response 在原字段基础上新增：
  - `effectiveBucket`
  - `availableBuckets`
  - `bucketLimitedToDaily`
- `bucket=1d` 路径的数据来源改为：
  - 在线 `codex_invocations`
  - `invocation_rollup_daily`（仅当请求时区与 Asia/Shanghai 归档日边界一致时合并）
  - `stats_source_deltas`（现有 CRS 增量）
- 当请求 bucket 小于 `1d` 且请求范围早于 `shanghai_retention_cutoff(INVOCATION_MAX_DAYS)` 时：
  - 后端必须以 `1d` 聚合返回；
  - `effectiveBucket=1d`
  - `availableBuckets=["1d"]`
  - `bucketLimitedToDaily=true`
- `firstByteSampleCount` / `firstByteAvgMs` / `firstByteP95Ms` 只由在线 success invocation 样本贡献；纯 rollup bucket 必须保持 `0 / null / null`。
- 当请求时区的自然日边界与 Asia/Shanghai 不一致时，daily timeseries 必须跳过 rollup rows，避免把 archived history 错桶到错误的本地日。

## 验收标准（Acceptance Criteria）

- Given 某个仍在 90d 范围内的历史日只存在 `invocation_rollup_daily`，When 请求 `range=90d&bucket=1d&timeZone=Asia/Shanghai`，Then 返回对应非零 bucket，且 first-byte 指标为 `0 / null / null`。
- Given 请求时区与 Asia/Shanghai 共享相同日边界（例如 `Asia/Singapore`），When 请求 daily timeseries，Then archived rollup day 仍会被合并到对应 bucket。
- Given 请求时区的自然日边界与 Asia/Shanghai 不一致（例如 `UTC`），When 请求 daily timeseries，Then 不得把 rollup rows 错投到该时区 bucket；此时 archived rollup rows 应被跳过。
- Given 较早日期来自 rollup、较近日期来自 live invocation，When 请求 daily timeseries，Then 两段历史连续可见，且总和与 `query_combined_totals(..., StatsFilter::All, InvocationSourceScope::All)` 一致。
- Given 同一天存在 rollup 与 live proxy invocation，When 请求 daily timeseries，Then 同一 bucket 会累加两部分 count/tokens/cost，且 first-byte 指标只来自 live success 样本。
- Given 同一天存在 `proxy` 与 `xy` rollup，When 以 `InvocationSourceScope::ProxyOnly` 查询 rollup helper，Then 只返回 `source='proxy'` 的记录。
- Given 请求 `range=30d&bucket=12h` 且范围早于 live subday window，When 请求 timeseries，Then 返回 `bucketSeconds=86400`、`effectiveBucket=1d`、`availableBuckets=["1d"]`，并包含对应 archived rollup day。
- Given 请求 `range=7d&bucket=12h` 且范围仍在 live subday window，When 请求 timeseries，Then 保持 `bucketSeconds=43200` 与 `effectiveBucket=12h`，不得无条件提升到 `1d`。
- Given 统计页进入归档窗口且旧状态残留 `12h` 等无效 bucket，When 页面拿到 timeseries 响应，Then bucket select 只展示 `1d`，并自动同步为 `1d`。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo test` 覆盖新增的 daily rollup continuity 用例。
- `cargo test` 覆盖新增的 archive-aware subday fallback 用例。
- `cd web && bun run test` 覆盖新增的 stats bucket helper 用例。
- `cargo check` 通过，且不引入新的 lint / 编译错误。

### Quality checks

- `cargo fmt --check` 无差异。
- 若需要 UI 证据，使用本地 smoke 验证 Dashboard / Stats 的 90d 历史恢复。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: `invocation_rollup_daily` range helper落地，并接入 daily timeseries 聚合。
- [x] M2: 子日级请求跨过 archive 边界时自动提升到 `1d`，并暴露实际生效 bucket 元信息。
- [x] M3: Rust + 前端回归测试覆盖 archive-aware bucket 行为与 stale bucket 回退。
- [x] M4: 验证、spec-sync、PR 与 review-loop 收敛。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：对于与 Asia/Shanghai 日边界不一致的时区，archived invocation days 仍无法按请求时区重分桶；本热修只保证不误投到错误日期。
- 风险：当浏览器时区与 Asia/Shanghai 日边界不一致时，前端可能只能看到 `1d` 且 archived day 仍为空；这是当前“避免错桶”的保守语义，不是新的数据丢失。
- 风险：CRS 增量按现有 captured-at day 聚合，热修不会改变它的统计边界。
- 开放问题：若未来需要跨任意时区恢复 archived daily history，需要引入可重分桶的 archive 粒度或额外维度。
- 假设：`invocation_rollup_daily.stats_date` 始终以 Asia/Shanghai 自然日落盘。

## 变更记录（Change log）

- 2026-03-11: 初始化 hotfix spec，冻结“daily timeseries after archive must stay continuous / no schema change / no detail backfill”范围。
- 2026-03-11: daily timeseries 已接入 `invocation_rollup_daily`，并补充 archived day、same-day mixed bucket、proxy-only scope 的后端回归测试。
- 2026-03-11: review-loop 发现 rollup 为 Asia/Shanghai 日粒度后，补充“仅在请求时区日边界匹配时合并 rollup”的保护逻辑与对应 UTC / Asia-Singapore 回归测试。
- 2026-03-11: 完成 shared testbox 生产快照验证，确认 Asia/Shanghai 日图恢复 archived rollup，UTC 等不匹配时区不会误并入 rollup。
- 2026-03-19: 扩展热修范围到归档窗口下的 subday bucket guard；`/api/stats/timeseries` 新增 `effectiveBucket` / `availableBuckets` / `bucketLimitedToDaily`，统计页据此自动限制 bucket 并回退 stale 选择。
- 2026-03-19: 本地验证通过，PR #184 已创建，当前收口到 `PR ready`。
