# 修复 legacy `http_200` success-like retention 漏清理（#erv4p）

## 状态

- Status: 已实现，待 PR / CI 收敛

## 背景 / 问题陈述

- 101 只读排查确认：`ai-codex-vibe-monitor-data` 当前约 `116.6G`，其中 `proxy_raw_payloads` 约 `110.5G`。
- `3-7` 天窗口内仍残留大量 `status=http_200`、`detail_level=full`、`error_message` 为空的历史 proxy 调用；这些记录语义上属于成功调用，但 retention 只按 `status='success'` 识别 structured prune，导致 raw 文件未进入清理链路。
- `#gwpsb` 已明确：真正失败的 `HTTP 200` Responses SSE 仍必须保持 `status=http_200` 并保留失败明细；这次 hotfix 只能兼容 legacy success-like 记录，不能把带失败证据的 `http_200` 误当成功。

## 目标 / 非目标

### Goals

- 将 legacy `http_200` 且 `trim(error_message)=''` 的记录纳入 success-like retention 语义。
- 统一 retention 与 `should_upgrade_to_upstream_response_failed()` 对 success-like 的基础判断，避免同一类历史记录在不同路径上语义漂移。
- 不改 public API、SQLite schema、运行配置、retention 窗口与 archive layout。

### Non-goals

- 不把所有 `2xx` 状态都扩成 success-like。
- 不修改 read-side rollups、proxy usage/cost backfill、统计接口或前端展示。
- 不包含 101 上的 live cleanup、deploy 或 merge。

## 功能与行为规格

- success-like 仅定义为：
  - `status=success`
  - `status=http_200 && trim(error_message)==''`
- `prune_old_invocation_details()` 在 `invocation_success_full_days..invocation_max_days` 窗口内，必须把 success-like 且 `detail_level=full` 的记录纳入 structured prune，清空 raw 路径并删除对应磁盘 raw 文件。
- `compress_cold_proxy_raw_payload_lane()` 必须沿用同一 success-like 语义，把已进入 structured prune 窗口的 success-like `full` 记录排除在冷压缩之外，避免先压缩后马上 prune。
- `should_upgrade_to_upstream_response_failed()` 必须复用同一 success-like helper，但保留现有 `existing_kind` 保护逻辑；真正带失败证据的 `http_200` 仍然走失败升级路径。
- 已经被 structured prune 的 archived row 在后续内部 replay 判定中，应继续被视为 success-like pruned detail，避免 legacy `http_200` 在 archive replay 环节重新绕过 payload-full guard。

## 验收标准

- Given 一条 `occurred_at` 落在 `invocation_success_full_days..invocation_max_days` 之间、`detail_level=full`、`status=http_200`、`error_message` 为空且带 raw path 的 legacy invocation，When 运行 retention live 模式，Then 该记录被标记为 `structured_only`，raw 路径清空且磁盘 raw 文件被删除。
- Given 一条 `status=http_200` 但 `error_message` 非空的记录，When 运行 retention，Then 该记录不会走 success-like structured prune，仍保留 failure 语义。
- Given 现有 `status=success` 的 success/full retention 用例，When 补丁落地后再次运行，Then 原有 structured prune、cold compression 与 archive 行为不回归。

## 验证

- `cargo test retention_prunes_old_success_invocation_details_and_sweeps_orphans -- --test-threads=1`
- `cargo test retention_prunes_old_legacy_http_200_success_like_invocation_details -- --test-threads=1`
- `cargo test retention_does_not_prune_legacy_http_200_rows_with_error_message -- --test-threads=1`
- `cargo test retention_compresses_cold_raw_payloads_and_updates_paths -- --test-threads=1`
- `cargo check`
