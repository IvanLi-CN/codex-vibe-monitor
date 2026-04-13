# raw 保真降本与历史维护追平 follow-up（#vw93e）

## 状态

- Status: 已实现，待 PR / CI / review-proof 收敛
- Created: 2026-04-13
- Last: 2026-04-13

## 背景 / 问题陈述

- 线上 `proxy_raw_payloads` 已经由成功请求的 full raw 主导，近实时排障确实依赖 raw，但当前“全部先写热明文，再等后续 maintenance 冷压缩”的策略会把磁盘与 retention backlog 一起推高。
- 现有 raw 读取链路（preview、详情、backfill、`search-raw`）已经完成 `identity | gzip` 透明兼容，但热路径仍缺少“写入时按大小直接选择 codec”的分层策略。
- `historicalRollupBackfill` 仍可能长期停在 `critical`，要求人工反复执行 `maintenance materialize-historical-rollups`，与“主库 hourly rollup 作为长期在线唯一真相源”的方向不一致。
- 对 `deactivated_workspace` / 结构化 `upstream_http_402` 这类明确上游拒绝账号，当前同轮 maintenance 还会继续 browser-UA fallback retry，且短时间内会被高频重复拉起，同步收益低、噪音高。

## 目标 / 非目标

### Goals

- 保留 raw 全量排障价值，不取消 raw 落盘，也不引入手动 pin。
- 新增 `PROXY_RAW_IMMEDIATE_GZIP_BYTES`，默认 `1 MiB`；大 raw 在首次写入时直接落为 gzip，小 raw 继续保留 plain 写入。
- 保持 preview、详情、backfill、`search-raw` 与 retention/archive 删除链路对 `.bin` / `.bin.gz` 的透明兼容。
- 继续沿用 `INVOCATION_SUCCESS_FULL_DAYS` / `INVOCATION_MAX_DAYS` 两段生命周期，不新增 failure-specific retention 配置。
- 让 startup/follow-up maintenance 以有界预算自动推进 legacy hourly rollup materialization，把 `historicalRollupBackfill` 从长期 `critical` 拉回可自愈状态。
- 对明确上游拒绝账号施加 6 小时 maintenance cooldown，并跳过同轮 browser-UA retry。

### Non-goals

- 不迁移 raw 到对象存储，不引入手动 pin 或额外归档层。
- 不改 HTTP / SSE / SQLite schema 的公开字段形状。
- 不做宿主机容量、系统参数或线上部署编排改造。

## 范围（Scope）

### In scope

- `src/proxy/raw_capture.rs`、`src/proxy/usage_persistence.rs` 的 raw 写盘 codec 选择、路径与元数据持久化。
- `src/maintenance/archive/cleanup.rs`、`src/maintenance/hourly_rollups.rs`、`src/maintenance/startup_backfill.rs` 的 bounded historical rollup auto-heal。
- `src/upstream_accounts/**` 中明确上游拒绝的 retry / cooldown 策略。
- `README.md`、`docs/deployment.md`、`docs/specs/README.md` 与本 spec 的同步。

### Out of scope

- 新增 UI 设置项、手动 pin 操作入口或线上部署执行。
- 改写 search-raw 的接口或引入新的全文索引系统。
- 改动 success/failure retention 的业务窗口定义。

## 需求（Requirements）

### MUST

- `PROXY_RAW_IMMEDIATE_GZIP_BYTES` 仅在 `PROXY_RAW_COMPRESSION=gzip` 时生效；默认值 `1048576`，配置 `0` 时完整回退到“热明文 + 冷压缩”路径。
- `>= threshold` 的 request/response raw 首次落盘直接生成 `.bin.gz`，`< threshold` 继续生成 `.bin`；`request_raw_codec` / `response_raw_codec` 与 `*_raw_path` 必须与实际磁盘文件一致。
- born-gzip 不得改变 `INVOCATION_SUCCESS_FULL_DAYS` / `INVOCATION_MAX_DAYS` 的现有 success/failure 生命周期语义；retention 对已是 gzip 的 live row 只做生命周期判断，不重复压缩。
- startup/follow-up maintenance 必须按固定 batch / 时间预算持续推进 legacy hourly rollup materialization，且读路径继续保持 query-only、fail-soft。
- `deactivated_workspace`、结构化 `upstream_http_402`、`upstream_rejected` 同轮不再触发 browser-UA fallback retry，并获得 6 小时 maintenance cooldown；401/403、429、普通 timeout 语义不变。

### SHOULD

- 为 born-gzip 写入、透明读取、bounded rollup catch-up 与 upstream-rejected cooldown 补齐 Rust 回归测试。
- 文档应明确新 env、自动自愈行为与必要时的手工 maintenance 兜底命令。

## 功能与行为规格（Functional/Behavior Spec）

### Raw capture / persistence

- 同步 raw 捕获与异步 streaming raw 持久化都按阈值决定 writer：小 payload 明文 `.bin`，大 payload 直接 gzip `.bin.gz`。
- `requestRawPath` / `responseRawPath` 继续视为 opaque path；调用方不得假定固定后缀。
- raw preview、详情页、backfill、`search-raw` 与 retention/orphan sweep 继续通过现有 codec-aware 读取/删除逻辑透明工作。

### Retention lifecycle

- success-like 记录仍在 `INVOCATION_SUCCESS_FULL_DAYS` 后进入 `structured_only`；任意记录超过 `INVOCATION_MAX_DAYS` 后归档/清理。
- 对 born-gzip live row，retention 只在需要 prune/archive/delete 时继续处理，不再把它当成待冷压缩对象重复压缩。

### Historical rollup auto-heal

- startup backfill 与后续 maintenance follow-up 每轮都可以在有界预算内推进 `materialize_historical_rollups`，优先消化 legacy archive pending backlog。
- backlog 未清空时，自动推进不应阻塞 `/health`；若 operator 需要一次性追平，仍可手动执行 `maintenance materialize-historical-rollups`。

### Upstream rejected maintenance policy

- 当 usage 抓取错误已明确指向 `deactivated_workspace` / `upstream_http_402` / `upstream_rejected` 时，本轮同步直接视为“明确上游拒绝”，跳过 browser-UA fallback retry。
- 这类账号落库时写入 6 小时 `cooldown_until`，maintenance 调度与执行阶段都要尊重该 cooldown，避免短期重复拉起。

## 接口契约（Interfaces & Contracts）

| Name | Kind | Scope | Change | Notes |
| --- | --- | --- | --- | --- |
| `PROXY_RAW_IMMEDIATE_GZIP_BYTES` | env | runtime | 新增 | 默认 `1048576`；`0` 禁用 born-gzip；仅在 `PROXY_RAW_COMPRESSION=gzip` 时生效 |
| `request_raw_path` / `response_raw_path` | DB/API field | existing | 兼容扩展 | 值可能为 `.bin` 或 `.bin.gz` |
| `request_raw_codec` / `response_raw_codec` | DB field | existing | 语义延续 | 继续显式区分 `identity | gzip` |
| `maintenance.historicalRollupBackfill` | stats | existing | 行为增强 | backlog 由 bounded auto-heal 持续推进，而不是长期依赖人工执行 |

## 验收标准（Acceptance Criteria）

- Given `PROXY_RAW_COMPRESSION=gzip` 且 raw 大小 `>= 1 MiB`，When request/response raw 首次落盘，Then 磁盘文件直接是 `.bin.gz`，且详情/preview/backfill/search-raw 都能透明读取。
- Given `PROXY_RAW_IMMEDIATE_GZIP_BYTES=0`，When raw 落盘，Then 行为完整回退到当前“热明文 + 冷压缩”路径。
- Given success-like 与 failure-like 记录，When retention 运行，Then `INVOCATION_SUCCESS_FULL_DAYS` / `INVOCATION_MAX_DAYS` 的现有生命周期保持不变，born-gzip 不新增额外 retention 分支。
- Given seeded legacy archive backlog，When startup/follow-up maintenance 持续执行，Then `historicalRollupBackfill.alertLevel` 最终可从 `critical` 降为非 critical，而无需依赖每次人工手动运行。
- Given 账号最近错误是 `deactivated_workspace` / `upstream_http_402`，When maintenance 同轮或 6 小时 cooldown 内再次评估该账号，Then 不会触发 browser-UA retry，也不会继续高频同步。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo check`
- `cargo test born_gzip -- --test-threads=1`
- `cargo test materialize_historical_rollups_marks_batches_and_prune_removes_files -- --test-threads=1`
- `cargo test fetch_usage_snapshot_skips_browser_user_agent_retry_for_upstream_rejected_402 -- --test-threads=1`
- `cargo test maintenance_plan_is_not_due_during_upstream_rejected_cooldown -- --test-threads=1`
- `cargo test record_pool_route_http_failure_marks_402_as_hard_error_and_records_reason -- --test-threads=1`
- `cargo test sync_triggered_402_summary_and_detail_export_as_upstream_rejected -- --test-threads=1`
- `scripts/shared-testbox-raw-smoke`

### UI / Storybook (if applicable)

- 不适用（后端与运维路径变更）

## 文档更新（Docs to Update）

- `README.md`
- `docs/deployment.md`
- `docs/specs/README.md`
- `docs/specs/vw93e-raw-born-gzip-rollup-followup/SPEC.md`

## Visual Evidence

- 不适用（本计划不涉及主人可见 UI 变更）

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 新增 born-gzip 阈值配置，并让 request/response raw 写盘阶段直接选择 plain/gzip writer
- [x] M2: 保持现有 retention 生命周期语义，并让 born-gzip 与透明读取/搜索链路兼容
- [x] M3: 让 startup/follow-up maintenance 有界自动推进 legacy hourly rollup materialization
- [x] M4: 为明确上游拒绝账号加入 no-UA-retry + 6h cooldown
- [ ] M5: 完成 PR / review-proof 收敛

## 方案概述（Approach, high-level）

- 在 raw 写入阶段优先做“按阈值选 codec”，避免把所有 payload 都先写热明文再等待后续压缩。
- 对 auto-heal 采用“小步持续推进”的 bounded batch 策略，让 backlog 在后台自愈，而不是把一次性追平责任全部丢给 operator。
- 对明确上游拒绝场景只做策略降频，不改变现有 401/403/429/timeout 的分类与优先级。

## 风险 / 假设（Risks, Assumptions）

- 假设 raw 继续全量保留，但 born-gzip 足以显著降低大 payload 的热存储膨胀；若后续流量继续上升，仍可能需要独立存储层 follow-up。
- 风险主要集中在 streaming raw writer 与 retention/born-gzip 兼容边界，因此必须依赖 targeted regression 与 shared-testbox smoke 收口。

## 变更记录（Change log）

- 2026-04-13: 创建 follow-up spec，冻结 born-gzip、bounded historical rollup auto-heal 与 upstream-rejected cooldown 的范围与验收。
- 2026-04-13: 完成 born-gzip raw capture、bounded historical rollup auto-heal、upstream-rejected 6h cooldown、README/deployment 同步，以及本地 + shared-testbox 验证。

## 实施结果

- `src/proxy/raw_capture.rs` 与 `src/proxy/usage_persistence.rs` 现在会按 `PROXY_RAW_IMMEDIATE_GZIP_BYTES` 在首次落盘时选择 plain / gzip writer，并同步持久化正确的 `*_raw_path` / `*_raw_codec`。
- `src/maintenance/archive/cleanup.rs`、`src/maintenance/hourly_rollups.rs`、`src/maintenance/startup_backfill.rs` 现在会在 startup/follow-up maintenance 中按有界预算持续推进 legacy hourly rollup materialization。
- `src/upstream_accounts/**` 现在把 `deactivated_workspace` / `upstream_http_402` / `upstream_rejected` 视为明确上游拒绝：同轮跳过 browser-UA fallback retry，并施加 6 小时 maintenance cooldown。

## 验证记录

- `cargo fmt --all`
- `cargo check`
- `cargo test born_gzip -- --test-threads=1`
- `cargo test materialize_historical_rollups_marks_batches_and_prune_removes_files -- --test-threads=1`
- `cargo test fetch_usage_snapshot_skips_browser_user_agent_retry_for_upstream_rejected_402 -- --test-threads=1`
- `cargo test maintenance_plan_is_not_due_during_upstream_rejected_cooldown -- --test-threads=1`
- `cargo test record_pool_route_http_failure_marks_402_as_hard_error_and_records_reason -- --test-threads=1`
- `cargo test sync_triggered_402_summary_and_detail_export_as_upstream_rejected -- --test-threads=1`
- `scripts/shared-testbox-raw-smoke`
  - run: `/srv/codex/workspaces/ivan/codex-vibe-monitor__40c096b7/runs/20260413_151705_shared_smoke_4b3bd8a6`
  - result: raw retention 仍能把 plain `.bin` 转成 `.bin.gz`，SQLite `request_raw_path` 同步更新，`search-raw` 同时命中 plain + gzip raw

## 参考（References）

- `docs/specs/t4v9k-retention-backlog-root-cause-fix/SPEC.md`
- `docs/specs/jg7a5-raw-payload-cold-compression-search/SPEC.md`
- `docs/specs/kfgvy-upstream-http-402-upstream-rejected/SPEC.md`
- `docs/specs/s6d1q-immutable-invocation-archive-segments/SPEC.md`
