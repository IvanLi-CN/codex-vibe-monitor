# Implementation

## 当前实现范围

- owner-facing 节点健康聚合已统一迁移到 `pool_upstream_request_attempts`。
- `Live`、`Settings`、forward-proxy timeseries、binding-node dialogs 复用同一真实节点尝试聚合路径。
- runtime endpoint key 与 canonical binding key 的映射已在 owner-facing 响应组装时显式处理，避免 runtime key / binding key 不一致造成的空计数或重复节点。
- `forward_proxy_attempts` / `forward_proxy_attempt_hourly` 仍保留给内部健康 / 探测 telemetry，不再参与 owner-facing 节点健康。
- `pool_upstream_request_attempts` archive writer 已补强：新归档直接写真实节点尝试行，避免生成 owner-facing 查询读不回来的空壳归档。
- schema 已补 `idx_pool_upstream_request_attempts_occurred_at_proxy_binding` 以支撑 owner-facing range + node 聚合查询。
- `pool_upstream_node_health_hourly_archive` 负责持久化每个 archive file 的真实节点小时桶；当 raw archive TTL 清理掉文件与 manifest 后，forward-proxy timeseries 改从这张长存小时表继续回放历史桶，避免 `90d` 等长时间窗在默认 retention 下断档。
- startup backfill 与 retention cleanup 现在会同时维护这张长存小时表：新归档在写 batch 时立即物化，旧 archive file 则由后台补齐，cleanup 只会在小时桶已物化后才允许删除 raw archive。
- retention 在向既有月归档 append 新 live ids 时，会区分“本次新建的月文件”与“已有旧行的月文件”：前者可以直接标记 owner-facing replay 完成，后者若之前尚未完整 materialize，则只追加本次 live rows/hours 并保持 archive 处于 pending，等待完整 backfill 覆盖整个月历史，避免把旧行误当成已回放。
- forward-proxy timeseries 对 retired node 的保留逻辑已扩展到 `Direct`：即便当前 `insert_direct=false`，只要窗口内还存在真实 `__direct__` 历史节点尝试，就继续输出 Direct 节点，并回退到稳定的 Direct label/source。

## 关键实现文件

- `src/forward_proxy/slices/storage_and_hourly_stats.rs`
- `src/schema.rs`
- `src/maintenance/archive/hourly_rollups.rs`
- `src/maintenance/archive/cleanup.rs`
- `src/maintenance/hourly_rollups.rs`
- `src/maintenance/retention.rs`
- `src/maintenance/archive/writers.rs`
- `src/tests/slices/broadcast_runtime_and_harness.rs`
- `src/tests/slices/time_and_proxy_basics.rs`
- `src/tests/slices/upstream_account_group_rules.rs`
- `src/upstream_accounts/tests_part_1.rs`
- `web/src/components/ForwardProxyLiveTable.stories.tsx`
- `web/src/components/LivePage.stories.tsx`
- `web/src/components/SettingsPage.stories.tsx`
- `web/src/components/UpstreamAccountGroupNoteDialog.stories.tsx`
- `web/src/components/storybookForwardProxyNodeHealth.ts`

## 口径细节

- 只读 `finished_at IS NOT NULL` 且 `proxy_binding_key_snapshot IS NOT NULL` 的终态节点尝试。
- `success` 记成功；其余终态记失败。
- `budget_exhausted_final` 视为池级收口，不计入节点健康。
- group 维度只保留作 binding-node 目录上下文，不再缩 owner-facing 计数范围。
- `weight24h` 保持旧契约，不与请求成功/失败口径联动变更。

## 验证状态

- Storybook parity stories 已补齐并回填 `SPEC.md` 的 `## Visual Evidence`。
- Rust 回归已覆盖：pending archive 直读、cache 去重、binding-node 全局口径，以及 raw archive cleanup 后 `90d` timeseries 仍保留真实节点小时历史。
- 额外回归已覆盖：同月月归档 append 会累计小时桶；对“模拟 rollout 前旧月文件”的 append 不会误写 fully replayed marker；Direct 在运行时关闭后仍保留 archived timeseries 历史。
