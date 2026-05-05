# OAuth 凭据 JSON 批量导入与验活 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/cgz7s-oauth-token-json-import/SPEC.md`

## Migrated History Notes

## Change log

- 2026-03-22：放宽导入文件中的 `expired` 契约；空值或缺失时允许回退到 `access_token.exp -> id_token.exp`，但非空无效 RFC3339 仍保持 `invalid`。
- 2026-03-19：补充导入路由专用 `32 MiB` body limit、前端 `100` 条分批验证/导入、共享测试机 413 复现结论与大请求 HTTP 回归要求。
- 2026-03-19：将验证阶段改为 `validation-jobs + SSE` 逐条实时返回，补充任务取消与“仅表体滚动”的对话框布局约束，并新增相关前后端回归测试。
- 2026-03-19：补充“导入阶段复用 validation job 缓存结果”的约束，避免 101 线上在 600+ 可导入账号场景下因重复 probe + 立即 sync 导致单次导入持续 30 分钟以上。
