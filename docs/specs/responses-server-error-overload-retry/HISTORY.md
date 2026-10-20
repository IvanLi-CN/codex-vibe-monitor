# Responses `server_error` 过载特征与同账号重试主题历史

> 这里记录主题生命周期、兼容性和背景；单次实现流水账保留在 `./IMPLEMENTATION.md` 或交付记录中。

## Lifecycle / Compatibility

- 2026-10-20: 创建 active canonical topic Spec，定义 `server_error` overload message 的窄范围识别和同账号重试合同。
- 2026-10-20: 在 `request_entry`、`stream_gate` 和 `failover` 实现 canonical `server_error` overload 的 same-account retry policy，并补充 Responses SSE/compact、预算耗尽、分类边界和 legacy overload 回归；Agent VM 中资源桶、check 和 Clippy 通过，stateful-sqlite profile 因缺少 `cargo-nextest` 未执行，线上验收待候选版本部署后完成。
- 2026-10-20: 在 Agent VM 安装 `cargo-nextest 0.9.148` 并执行 stateful-sqlite profile；两次完整运行分别为 1505/1506 与 1504/1506 通过，失败项均为既有时序敏感用例且隔离复跑通过；本主题新增 Responses overload 测试通过，线上验收仍待候选版本部署。
- 2026-10-20: 主人明确修正本次交付口径：线上 empirical acceptance 不作为前置 gate；以本地单元/集成测试验证逻辑正确性，继续进入 PR 与发布流程；主题 Spec 的长期行为合同不因此扩展。
- 2026-10-20: 修复批次此前在 Agent VM 完成 `routing_timeout_and_overload_failover` 37/37、`stateful-sqlite` 1508/1508、`cargo check --locked --all-targets --all-features` 和 `cargo clippy --locked --all-targets --all-features -- -D warnings`；随后 review 发现并修复了混合失败与累计 timeout 边界。
- 2026-10-21: 修复最终 review 发现的混合失败边界：canonical `server_error` signal 激活后持续锁定同账号策略，429/413、transport、handshake 和首 chunk 重试共享 4 次预算及累计 timeout；专项路由桶 38/38 通过，完整 stateful profile 首轮的 11 个既有时序敏感失败逐项复跑 11/11 通过，check 和 Clippy 通过。
- 2026-10-21: 补齐同账号重试的 forward-proxy channel 固定回归和 canonical + 413 + 429 混合失败回归；最终 `routing_timeout_and_overload_failover` 资源桶以单线程方式 38/38 通过，check 和 Clippy 继续通过。
- 2026-10-21: 修复前置 429 扩展 Responses retry loop 的预算边界；Responses family 的 429/413 与 canonical `server_error` 共享 4 次总 attempt 预算，并新增前置 429 回归。补充内部标点/空白归一化分类样例后，专项资源桶以单线程方式 39/39 通过。
- 2026-10-21: 按主人确认将本地单元/集成测试作为本次逻辑验收依据；线上 empirical observation 不再作为发布前置 gate。
- 既有 `server_is_overloaded` 与 concurrency `rate_limit_exceeded` overload 行为继续由历史 Responses overload specs 拥有，本主题不替代其账号/route 切换语义。

## Replacements / Background

- 主题源于线上 HTTP 200 `response.failed` 样本：错误码为 `server_error`，消息表达上游服务过载，但当前实现只记录一次 attempt 并返回 `service_failure`。
- 本主题沿用 `bk2pt` 的 early gate、`p6x4r` 的 Responses retry budget 和 `br38t` 的 downstream-visible replay boundary，只新增一个错误特征与其不切换上游的策略。

## Related Changes

- None

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
