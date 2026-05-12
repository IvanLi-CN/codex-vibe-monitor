# 请求日志可观测性增强（IP / Cache Tokens / 分阶段耗时 / Prompt Cache Key） - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/z9h7v-invocation-log-observability/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-02-25: 初始化规格，冻结实现边界与验收口径。
- 2026-02-25: 完成后端字段采集、`/api/invocations` 投影扩展与前端表格升级，并通过 `cargo test`、`cargo check`、`web npm run build` 验证。
- 2026-02-25: 修复 SSE `records` 广播回查 SQL 投影不全问题，确保 `endpoint/requesterIp/promptCacheKey/failureKind` 与 `/api/invocations` 一致，并补充回归测试。
- 2026-02-25: 将对外字段从 `codexSessionId` 切换为 `promptCacheKey`，新增启动期历史数据全量回填与旧键清理，并补充回填幂等/异常分支测试。
- 2026-02-25: 修复启动回填对历史相对路径 raw 文件的兼容性（新增 `database_path` 父目录兜底），避免因工作目录变化导致 `skipped_missing_file` 异常偏高。
- 2026-05-11: 修正代理诊断展示口径：调用详情隐藏 `source` 且不再作为代理名 fallback，号池尝试明细展示实际落库的 `proxyBindingKeySnapshot`。
- 2026-05-11: 将号池 `budget_exhausted_final` 合成终态从真实重试明细中拆出，明确展示未发起新的上游请求，避免误读为同账号 429 后再次重试。
- 2026-05-12: 修正号池尝试代理字段可读性：前端使用 `proxyBindingKeySnapshot` 查询绑定节点并展示代理显示名，未解析时才显示紧凑 key。
