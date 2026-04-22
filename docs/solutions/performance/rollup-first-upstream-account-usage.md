# Rollup-first 上游账号 usage 读取模式

## 适用场景

- 列表页需要展示账号窗口 usage，但首屏预算只有几十毫秒。
- 历史 usage 已经过 retention / archive，继续在线扫明细会显著拖慢交互接口。
- 前端可以接受“列表先出，usage 后补”的两段式体验。

## 核心结论

- 列表接口不要同步计算 usage。
- usage 查询优先读 hourly rollup，再用最小范围 raw fallback 补齐缺口。
- 交互链路里的 usage 查询应该固定走 `hourly rollup + uncovered-bucket replay + partial boundary replay`。

## 推荐模式

### 1. 轻量 roster 与 usage hydrate 拆分

- roster 只返回列表骨架所需字段：账号摘要、分组、代理 catalog、分页和 metrics。
- usage 单独提供批量 hydrate endpoint，按当前页或当前可见账号请求。
- 前端维护 query generation、已 hydrate 集合与 pending 集合，丢弃 stale usage 响应。

### 2. rollup-first 查询顺序

1. 完整整点区间读取 hourly rollup。
2. 若 deploy / upgrade 后某些 `(account_id, bucket_start_epoch)` 还没被 hourly 覆盖，只回补这些未覆盖 bucket。
3. retention 边界 partial bucket 再回读 raw rows。
4. raw fallback 必须按 `(account_id, bucket_epoch)` 去重，保证已有 hourly bucket 不会被重复累计。

### 3. 为什么不要在列表接口里算 usage

- 首屏列表会被 usage 聚合拖到和账号数线性相关。
- grouped / grid 的 `includeAll=true` 会把全量账号一起放大成秒级阻塞。
- archive-sensitive 查询一旦进入交互链路，会把本该稳定的 UI 预算交给磁盘和解压路径。

## 实施要点

- 为业务维度单独建 hourly rollup 表时，直接复用统一的 live replay / historical materialization 机制。
- 列表 handler 要显式埋点 `roster_core_ms` 与 `usage_batch_ms`，否则很难确认 `10ms` 目标究竟卡在哪一段。
- Storybook / mock runtime 也要模拟真实接口语义：roster 不带 `actualUsage`，batch endpoint 再补 usage，避免 UI 证据失真。

## 常见坑

- 只改后端而不改 Storybook mock，会让本地 stories 继续假装 roster 自带 usage。
- grouped/grid 若直接 hydrate 全量账号，只是把慢请求从 GET 挪到了 POST，没有真正解决问题。
- 只看 live cursor 就相信小时桶完整，会在 fresh deploy / upgrade 时把 full-hour usage 漏掉。
- stale hydrate 请求如果在 `finally` 里直接清空 pending ids，会把更新一轮的真实请求误判成“已完成”。
- 列表 query key 变化时如果不清空 hydrate generation，旧 usage 响应会覆盖新筛选结果。

## 何时不适用

- 页面必须在首屏一次性展示精确 usage 且无法接受占位态。
- 数据总量很小且没有 retention / archive，列表同步 enrich 成本已经稳定低于预算。
- 业务要求明细级 drilldown，而不是列表级聚合展示。
