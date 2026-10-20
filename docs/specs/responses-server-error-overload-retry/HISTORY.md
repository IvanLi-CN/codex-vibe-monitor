# Responses `server_error` 过载特征与同账号重试主题历史

> 这里记录主题生命周期、兼容性和背景；单次实现流水账保留在 `./IMPLEMENTATION.md` 或交付记录中。

## Lifecycle / Compatibility

- 2026-10-20: 创建 active canonical topic Spec，定义 `server_error` overload message 的窄范围识别和同账号重试合同。
- 2026-10-20: 在 `request_entry`、`stream_gate` 和 `failover` 实现 canonical `server_error` overload 的 same-account retry policy，并补充 Responses SSE/compact、预算耗尽、分类边界和 legacy overload 回归；Agent VM 中资源桶、check 和 Clippy 通过，stateful-sqlite profile 因缺少 `cargo-nextest` 未执行，线上验收待候选版本部署后完成。
- 既有 `server_is_overloaded` 与 concurrency `rate_limit_exceeded` overload 行为继续由历史 Responses overload specs 拥有，本主题不替代其账号/route 切换语义。

## Replacements / Background

- 主题源于线上 HTTP 200 `response.failed` 样本：错误码为 `server_error`，消息表达上游服务过载，但当前实现只记录一次 attempt 并返回 `service_failure`。
- 本主题沿用 `bk2pt` 的 early gate、`p6x4r` 的 Responses retry budget 和 `br38t` 的 downstream-visible replay boundary，只新增一个错误特征与其不切换上游的策略。

## Related Changes

- None

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
