# VLESS XHTTP Forward Proxy Support 实现状态

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: implemented
- Lifecycle: active
- Catalog note: VLESS XHTTP 支持已接入共享构建、身份规范化、验证错误合同和前端 API 归一化；确定性验证、Xray 配置预检和受控 exact-head smoke 已通过，等待最终 CI 与合并门禁。

## Implementation Coverage

- Requirement coverage: `REQ-VLESS-XHTTP-001` 至 `REQ-VLESS-XHTTP-010` 已由 XHTTP stream-settings 构建、规范化身份、临时 Xray 验证、稳定错误码、API 字段保留和回归测试覆盖。
- Backend: VLESS 独占处理 `type/net=xhttp`，支持 `none`、`tls`、`reality`、`host`、`path`、四种受支持 mode 和 object 类型 `extra`；输出保留 `xhttpSettings.extra`，顶层字段覆盖其中同名项，错误不会降级为 TCP 或直连。
- Identity: XHTTP 的有效字段和递归稳定化后的 `extra` 进入 canonical identity；查询顺序、别名和等效编码不会制造重复节点，Reality `alpn` 保留在身份但不写入 `realitySettings`。
- Validation: 单节点和订阅验证返回稳定 `code`，保留既有响应字段；配置、启动、连接、超时和 upstream 状态失败均保持失败，批量验证继续遵守最多 12 轮及任一轮成功规则。
- Frontend: API normalization 和 Settings 状态边界保留 `code`、`ok`、`message`、`normalizedValue`、`discoveredNodes` 和 `latencyMs`；未改变 Settings 页面布局和既有状态机。
- Sensitive-value boundary: `normalizedValue` 仍只用于确认后保存规范化节点，Settings 验证成功提示不再渲染完整分享链接；运行时、实时统计和历史统计只返回协议/主机/端口级安全端点标识，非 2xx API 异常也保留稳定 `code` 到批量结果。
- Binding safety: 后端继续保留完整旧 URL 作为内部兼容解析别名，但绑定节点 API 的 `aliasKeys` 只返回 opaque key（包括 `fpn_`、`fpb_` 和兼容的安全历史 key），不暴露 URL、UUID 或 XHTTP `extra`。
- Runtime classification: Xray 启动前先执行配置预检；配置被二进制拒绝时返回 `xray_config_error`，无法启动二进制时仍返回 `xray_startup_error`。XHTTP 延迟探测使用验证允许状态，旧协议保留既有延迟状态兼容规则。
- Verification commands: 本地 `cargo fmt --all -- --check`、`git diff --check` 和 `cargo metadata --locked --no-deps` 已通过；本地 macOS `cargo test` 在编译前被 `sqlx_macros` 动态库的 `mis-aligned LINKEDIT string pool` 阻断，不能把它记为测试通过。PR CI 的后端资源档案、前端测试、构建和 smoke image 是仓库级门禁，其中 `stateful-sqlite` 只表示全局回归，不是 XHTTP 专项证据。
- Rollout facts: 本次只验证脱敏的本地合成节点和本地 upstream，不连接线上节点、不改线上持久配置，也不提交真实凭据。当前实现继续只改变仓库运行时构建规则，并在 Xray 拒绝配置时显式失败，不协议降级或直连回退。
- Empirical smoke evidence: 使用当前 PR 构建出的 Linux 后端、同一构建流程提供的 Xray 26.2.6、合成 VLESS XHTTP server 和本地只读 upstream；`xray run -test` 通过，正向验证返回成功且 upstream 收到请求，停止 Xray 后同一验证返回 `proxy_connect_error` 且 upstream 计数不增加，临时 Xray route 文件清理完毕。证据只记录状态、阶段和版本，不保存节点 URL、UUID 或其他凭据。

## Coverage / rollout summary

- 受控配置验证覆盖 XHTTP 的 security、mode、顶层字段优先级、嵌套 `extra`、Reality ALPN 归属、身份等价/差异和稳定错误码；测试 fixture 使用脱敏值。
- Agent VM 用于执行 Linux smoke 产物，不作为 Rust 资源档案的替代；仓库级 Rust 回归以 PR CI 的 exact-head 结果为准。

## Remaining Gaps

- Clippy 修复后的 exact-head CI、合并前置检查和 PR 合并仍待完成；在此之前不宣称 release-ready 或已合并。

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
