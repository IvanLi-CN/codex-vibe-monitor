# Daily timeseries archive continuity hotfix（#nm7ep）

## 状态

- Status: 进行中（2/3）
- Created: 2026-03-11
- Last: 2026-03-11

## 背景 / 问题陈述

- `RETENTION_ENABLED=true` 与 `INVOCATION_MAX_DAYS=7` 的线上误配置把大量历史调用明细归档出主库后，`/api/stats/timeseries?bucket=1d` 只读取在线 `codex_invocations`，导致 90d UsageCalendar / Stats daily chart 出现连续历史断层。
- `/api/stats` 与 `summary?window=all` 已经会合并 `invocation_rollup_daily`，所以当前故障表现为“总量还在，但按日图表历史变空”。
- 需要以热修方式补齐 daily timeseries 的归档连续性，而不恢复在线 invocation 明细，也不改现有 API schema。

## 目标 / 非目标

### Goals

- 让 `bucket=1d` 的 `/api/stats/timeseries` 在 invocation 明细被 archive 后，继续从 `invocation_rollup_daily` 读取对应历史日桶。
- 保持现有 `TimeseriesResponse` / `TimeseriesPoint` JSON 字段不变。
- 保证纯 rollup 日桶只回填 count/tokens/cost，不伪造 first-byte latency 统计。
- 增加后端回归测试，覆盖 archived rollup day、live + rollup continuity、proxy-only source scope。

### Non-goals

- 不恢复在线 `codex_invocations` 明细，不回灌 archive sqlite.gz。
- 不改 summary all、SSE、非 daily bucket、前端 API 类型或 UI 交互。
- 不在本 PR 内直接改线上部署或要求生产重启。

## 范围（Scope）

### In scope

- `src/stats/mod.rs`：新增按日期范围读取 `invocation_rollup_daily` 的 helper，并复用 `InvocationSourceScope` 过滤。
- `src/api/mod.rs`：改造 `fetch_timeseries_daily`，合并 rollup rows、在线 invocation rows 与现有 CRS deltas。
- `src/tests/mod.rs`：新增/调整 Rust 测试覆盖 archive continuity 与 rollup-only bucket 行为。
- `docs/specs/README.md`：登记本热修 spec 并在 PR 收敛阶段同步状态。

### Out of scope

- 数据迁移、schema 变更。
- 前端组件改造。
- 生产明细恢复脚本。

## 接口契约（Interfaces & Contracts）

- `GET /api/stats/timeseries` 保持原 query / response 不变。
- `bucket=1d` 路径的数据来源改为：
  - 在线 `codex_invocations`
  - `invocation_rollup_daily`（仅当请求时区与 Asia/Shanghai 归档日边界一致时合并）
  - `stats_source_deltas`（现有 CRS 增量）
- `firstByteSampleCount` / `firstByteAvgMs` / `firstByteP95Ms` 只由在线 success invocation 样本贡献；纯 rollup bucket 必须保持 `0 / null / null`。
- 当请求时区的自然日边界与 Asia/Shanghai 不一致时，daily timeseries 必须跳过 rollup rows，避免把 archived history 错桶到错误的本地日。

## 验收标准（Acceptance Criteria）

- Given 某个仍在 90d 范围内的历史日只存在 `invocation_rollup_daily`，When 请求 `range=90d&bucket=1d&timeZone=Asia/Shanghai`，Then 返回对应非零 bucket，且 first-byte 指标为 `0 / null / null`。
- Given 请求时区与 Asia/Shanghai 共享相同日边界（例如 `Asia/Singapore`），When 请求 daily timeseries，Then archived rollup day 仍会被合并到对应 bucket。
- Given 请求时区的自然日边界与 Asia/Shanghai 不一致（例如 `UTC`），When 请求 daily timeseries，Then 不得把 rollup rows 错投到该时区 bucket；此时 archived rollup rows 应被跳过。
- Given 较早日期来自 rollup、较近日期来自 live invocation，When 请求 daily timeseries，Then 两段历史连续可见，且总和与 `query_combined_totals(..., StatsFilter::All, InvocationSourceScope::All)` 一致。
- Given 同一天存在 rollup 与 live proxy invocation，When 请求 daily timeseries，Then 同一 bucket 会累加两部分 count/tokens/cost，且 first-byte 指标只来自 live success 样本。
- Given 同一天存在 `proxy` 与 `xy` rollup，When 以 `InvocationSourceScope::ProxyOnly` 查询 rollup helper，Then 只返回 `source='proxy'` 的记录。
- Given 非 daily bucket（例如 `15m` / `1h`），When 请求 timeseries，Then 现有行为不变。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo test` 覆盖新增的 daily rollup continuity 用例。
- `cargo check` 通过，且不引入新的 lint / 编译错误。

### Quality checks

- `cargo fmt --check` 无差异。
- 若需要 UI 证据，使用本地 smoke 验证 Dashboard / Stats 的 90d 历史恢复。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: `invocation_rollup_daily` range helper 落地，并接入 daily timeseries 聚合。
- [x] M2: Rust 回归测试覆盖 archived rollup day、boundary-matched timezone、timezone mismatch skip、continuity、same-day mixed bucket、proxy-only scope。
- [ ] M3: 验证、spec-sync、PR 与 review-loop 收敛。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：对于与 Asia/Shanghai 日边界不一致的时区，archived invocation days 仍无法按请求时区重分桶；本热修只保证不误投到错误日期。
- 风险：CRS 增量按现有 captured-at day 聚合，热修不会改变它的统计边界。
- 开放问题：若未来需要跨任意时区恢复 archived daily history，需要引入可重分桶的 archive 粒度或额外维度。
- 假设：`invocation_rollup_daily.stats_date` 始终以 Asia/Shanghai 自然日落盘。

## 变更记录（Change log）

- 2026-03-11: 初始化 hotfix spec，冻结“daily timeseries after archive must stay continuous / no schema change / no detail backfill”范围。
- 2026-03-11: daily timeseries 已接入 `invocation_rollup_daily`，并补充 archived day、same-day mixed bucket、proxy-only scope 的后端回归测试。
- 2026-03-11: review-loop 发现 rollup 为 Asia/Shanghai 日粒度后，补充“仅在请求时区日边界匹配时合并 rollup”的保护逻辑与对应 UTC / Asia-Singapore 回归测试。
