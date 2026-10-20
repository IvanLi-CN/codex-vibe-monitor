# VLESS XHTTP Forward Proxy Support 实现状态

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: implemented
- Lifecycle: active
- Catalog note: VLESS XHTTP 支持已接入共享构建、身份规范化、验证错误合同和前端 API 归一化；等待受控 live smoke 与交付审查。

## Implementation Coverage

- Requirement coverage: `REQ-VLESS-XHTTP-001` 至 `REQ-VLESS-XHTTP-010` 已由 XHTTP stream-settings 构建、规范化身份、临时 Xray 验证、稳定错误码、API 字段保留和回归测试覆盖。
- Backend: VLESS 独占处理 `type/net=xhttp`，支持 `none`、`tls`、`reality`、`host`、`path`、四种受支持 mode 和 object 类型 `extra`；输出保留 `xhttpSettings.extra`，顶层字段覆盖其中同名项，错误不会降级为 TCP 或直连。
- Identity: XHTTP 的有效字段和递归稳定化后的 `extra` 进入 canonical identity；查询顺序、别名和等效编码不会制造重复节点，Reality `alpn` 保留在身份但不写入 `realitySettings`。
- Validation: 单节点和订阅验证返回稳定 `code`，保留既有响应字段；配置、启动、连接、超时和 upstream 状态失败均保持失败，批量验证继续遵守最多 12 轮及任一轮成功规则。
- Frontend: API normalization 保留 `code`、`ok`、`message`、`normalizedValue`、`discoveredNodes` 和 `latencyMs`；未改变 Settings 页面布局和既有状态机。
- Verification commands: `cargo fmt --all -- --check`、Rust 定向测试、`bash .github/scripts/run-backend-tests.sh --profile lightweight`、`bash .github/scripts/run-backend-tests.sh --profile stateful-sqlite`、`cargo check --locked --all-targets --all-features`、`cargo clippy --locked --all-targets --all-features -- -D warnings`、`cd web && bun run test`、`bun run typecheck:web` 和 `cd web && bun run build` 已通过。
- Rollout facts: 当前线上验证曾确认 VLESS XHTTP 节点被解析但旧临时 Xray 配置缺少 XHTTP 设置；本实现仅改变仓库运行时构建规则，不改线上持久配置。

## Coverage / rollout summary

- 受控配置验证覆盖 XHTTP 的 security、mode、顶层字段优先级、嵌套 `extra`、Reality ALPN 归属、身份等价/差异和稳定错误码；测试 fixture 使用脱敏值。
- Agent VM 已检查但当前镜像没有 Rust 工具链，因此 Rust 资源档案在本机使用仓库脚本完成；不把 VM 缺少工具链当作实现缺陷。

## Remaining Gaps

- 尚未完成受控 live smoke verification：需要一次临时或脱敏的真实 VLESS XHTTP/Reality 节点，观察 configured upstream 的实际路由结果；本地测试不替代该观察。
- 交付审查、提交和 PR 发布仍待完成；在此之前不宣称 release-ready 或已合并。

## Related Changes

- `src/forward_proxy/slices/manager_xray_types_and_tests.rs`
- `src/forward_proxy/slices/storage_and_hourly_stats/probe_and_validation.rs`
- `src/forward_proxy/slices/settings_validation_and_runtime_sync.rs`
- `src/forward_proxy/mod.rs`
- `src/share_links.rs`
- `web/src/lib/api/core-foundation.ts`
- `web/src/lib/api.test.ts`
- `src/tests/lightweight/forward_proxy_config_and_storage.rs`

## References

- `./SPEC.md`
- `./HISTORY.md`
