# 移除直连反向代理并为号池接入分组绑定正向代理（#mww8f）

## 状态

- Status: 已实现，待截图提交授权 / PR 收敛
- Created: 2026-03-26
- Last: 2026-03-27

## 背景

- 当前 `/v1/*` 同时承载“号池路由”与“直连反向代理”两条执行链，导致缺少号池路由 key 的请求仍会回退到全局 `OPENAI_UPSTREAM_BASE_URL`。
- forward proxy 当前仍保留 synthetic `direct` 节点，账号上下文请求并不保证一定经过真实代理节点。
- 分组元数据目前只有共享备注，缺少“按分组硬绑定代理节点”的运行时约束，无法让同组账号稳定共用一个代理节点并在连续网络失败后切换。
- 线上复核显示分组绑定弹窗仍暴露原始订阅地址，并且长标题与长列表会把节点卡片撑高、挤掉 footer，导致小窗口里无法完整看到选项与保存按钮。
- 当前分组绑定列表没有显式 `Direct` 选项，也没有统一的协议类型标签，用户无法判断节点协议，也无法把直连作为硬绑定池的一部分。

## 目标 / 非目标

### Goals

- 保留 `/v1/*` 入口，但移除非号池直连执行路径；未命中号池路由 key 时统一返回 `401` JSON 错误。
- 所有账号上下文上游请求都必须经过真实 forward proxy，默认使用全局自动路由。
- 为已存在账号分组增加 `boundProxyKeys` 元数据，并在分组设置中支持多选绑定代理节点。
- 分组绑定多个节点时，组内共享“当前节点 + 连续网络失败计数”；连续 `3` 次网络失败后随机切到另一个已绑定节点，不使用权重。
- 删除 legacy reverse-proxy 设置面与 `/api/settings` 中的 `proxy` 配置块，同时移除 `insertDirect` 前后端契约。
- 分组绑定弹窗不得显示订阅地址或内部 key，只显示截断标题、协议类型与 24 小时请求趋势。
- 分组绑定弹窗必须具备独立滚动层，长列表下 footer 始终保持在视口内。
- `Direct` 仅在分组绑定路径中作为显式可选项恢复，可与代理节点混选；未绑定分组继续维持当前自动路由语义，不把 direct 重新混回全局 automatic 候选池。

### Non-goals

- 不移除 `/v1/*` 入口本身，也不改动号池 sticky、tag、账号健康与 attempts 观测语义。
- 不把 `auth.openai.com` token exchange / refresh 或 `moemail` 请求纳入分组绑定代理范围。
- 不做 `proxy_model_settings`、legacy SQLite 行或历史调用记录的 destructive cleanup / 迁移。

## 范围

### In scope

- `src/main.rs`：移除 `/v1/*` 非号池直连分支，并把 pool live request / OAuth live bridge 接入分组感知的 forward-proxy 选路。
- `src/forward_proxy/mod.rs`：移除全局 reverse-proxy / `insertDirect` 合约，保留仅供分组绑定使用的 synthetic `direct` 运行时节点，新增“全局自动路由 + 分组硬绑定路由”双通道，以及组级连续网络失败切换状态。
- `src/upstream_accounts/mod.rs`：扩展分组 metadata 存储与接口，列表返回 `boundProxyKeys` 和可绑定节点清单，并把 usage/manual sync/import validation 接入代理感知发送层。
- `web/src/pages/Settings.tsx`、`web/src/hooks/useSettings.ts`、`web/src/lib/api.ts`：删除 reverse-proxy 设置契约，仅保留 forward proxy 与 pricing。
- `web/src/pages/account-pool/**`、`web/src/components/**`、`web/src/lib/upstreamAccountGroups.ts`：把“分组备注”升级为“分组设置（备注 + 代理节点多选）”，并修复长标题、协议标签、直连选项与列表滚动布局。
- `docs/specs/README.md`：登记本工作项。

### Out of scope

- 新增独立的分组管理页面或分组重命名能力。
- 调整 `routeMode=forward_proxy` 历史展示或清理 legacy DB 字段。
- 为尚未落库的新分组草稿增加独立的代理绑定持久化协议。

## 功能规格

### `/v1/*` 新语义

- `request_matches_pool_route(...) == false` 时，`/v1/*` 统一返回 `401` JSON 错误，不再构建全局上游直连请求。
- `GET /v1/models` 仅允许号池执行链处理，不再使用 legacy hijack / merge-upstream 设置。
- `/v1/*` 相关 capture / invocation 记录不得再新增“非号池直连反代”调用。

### Forward proxy 运行时

- `ForwardProxySettings`、`ForwardProxySettingsResponse` 与前端 `ForwardProxySettings` 不再包含 `insertDirect`。
- 全局 Settings / live proxy inventory / timeseries 响应不再暴露 synthetic `direct` 节点。
- 分组绑定节点响应新增 `protocolLabel`，取值固定为 `DIRECT / HTTP / HTTPS / SOCKS5 / SOCKS5H / VMESS / VLESS / TROJAN / SS / UNKNOWN`。
- synthetic `direct` 仅在分组绑定路径中作为显式候选节点返回，key 固定为 `__direct__`，显示文案固定为 `Direct / DIRECT`。
- 未分组或分组未绑定节点的账号，继续走现有全局自动路由。
- 已绑定分组的账号，只能在 `boundProxyKeys` 仍然存在且 `selectable=true` 的节点集合内路由；若可选集合为空，则本次账号尝试立即失败，不回退到全局池。
- 组内状态按 `group_name` 共享，维护：
  - 当前代理节点 key
  - 连续网络失败次数
- 任一成功到达上游的请求都会把该组当前节点的连续网络失败计数清零。
- 只有以下失败会累加连续失败计数：
  - send / connect error
  - handshake timeout
  - stream-before-success error
- HTTP `429/4xx/5xx` 不推进切换计数，也不改变组内当前节点；它们只沿用现有账号级 failover / cooldown 逻辑。
- 当组内当前节点连续 `3` 次网络失败且绑定集合存在其他可选节点时，运行时随机切到另一个绑定节点，并从 `0` 重新计数。

### 分组元数据与接口

- 继续复用 `pool_upstream_account_group_notes` 作为 group metadata 存储，新增 `bound_proxy_keys_json` 列，以 `group_name` 为主键。
- `GET /api/pool/upstream-accounts` 响应中的 `groups[]` 每项至少包含：
  - `groupName`
  - `note`
  - `boundProxyKeys`
- `GET /api/pool/upstream-accounts` 额外返回 `forwardProxyNodes[]`，每项至少包含：
  - `key`
  - `displayName`
  - `protocolLabel`
  - `source`
  - `penalized`
  - `selectable`
- `PUT /api/pool/upstream-account-groups/:groupName` 支持更新 `note` 与 `boundProxyKeys`；若分组不存在实际账号，返回 `404`。
- 已保存但当前 inventory 已不存在的 `boundProxyKeys` 会继续保留在分组 metadata 中；前端需标记为失效，运行时只使用仍然 `selectable=true` 的 key。

### 分组绑定弹窗

- 节点标题必须单行省略，hover/title 保留完整 `displayName`。
- 节点副信息只显示协议类型标签，不显示 `node.key`、订阅 URI 或其他原始地址。
- 弹窗 body 与绑定节点列表必须拆成两层滚动：header/footer 固定，节点列表在 `9+` 个节点时独立滚动。
- `Direct` 可与其他代理节点同时选中，并和其他已绑定节点一起参与组内连续网络失败后的随机切换。

### 账号上下文请求接入代理

- 以下请求必须按账号所属分组解析 forward-proxy scope，并使用真实代理节点发出：
  - pool live request（API Key）
  - pool live request（OAuth bridge）
  - usage snapshot
  - manual sync
  - imported OAuth validation
- `auth.openai.com` token exchange / refresh 与 `moemail` 保持现状，不接入分组绑定代理。

## 验收标准

- Given 请求未携带有效号池路由 key，When 访问 `/v1/*`，Then 服务返回 `401` JSON 错误，且不会再向全局上游发起直连反代请求。
- Given forward proxy inventory 为空，When 号池账号请求尝试连上游，Then 请求失败且不会回退到 `direct`。
- Given 账号未分组或所属分组未绑定节点，When 发起上游请求，Then 继续使用全局自动路由。
- Given 分组绑定了多个代理节点，When 当前节点连续发生 `3` 次 send/connect/handshake/stream-before-success 网络失败，Then 组内当前节点随机切换到另一个已绑定节点。
- Given 分组绑定节点返回 `429/4xx/5xx`，When 请求完成，Then 组内当前节点不切换，连续网络失败计数被清零。
- Given 分组保存了不存在或不可选的节点 key，When 运行时解析绑定集合，Then 仅使用仍然 `selectable=true` 的 key；若一个也没有，则本次账号尝试立即失败。
- Given 打开号池分组设置，When 用户查看绑定节点列表，Then 页面不显示任何 `ss://`、`vless://`、`vmess://`、`trojan://`、`http://`、`https://` 原始订阅地址，只显示截断标题和协议类型。
- Given 打开号池分组设置且存在 `9+` 个节点，When 用户浏览节点列表，Then footer 仍然可见，节点区内部可滚动，页面本身不需要额外滚动。
- Given 打开号池分组设置，When 用户多选 `Direct` 与其他绑定节点并保存，Then 刷新后 `boundProxyKeys` 稳定回显 `__direct__` 与其他绑定 key，且 Storybook 至少覆盖：
  - 长订阅 URI 不外露
  - `9+` 节点时列表内滚动且 footer 可见
  - `Direct` 与代理混选
  - 无绑定自动路由
  - 硬绑定多节点
  - 绑定节点缺失/不可用

## 质量门槛

- `cargo fmt`
- `cargo check`
- `cargo test`
- `cd web && bun run test`
- `cd web && bun run build`
- `cd web && bun run storybook:build`

## Visual Evidence

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  sensitive_exclusion: N/A
  submission_gate: pending-owner-approval-for-push
  story_id_or_title: Account Pool/Components/Upstream Account Group Settings Dialog/Hard Bound Multiple Nodes
  state: direct-protocol-scroll-layout
  evidence_note: 验证分组绑定弹窗在桌面宽度下展示 `Direct`、协议标签和右侧 24 小时趋势；长标题被截断，底部操作栏始终可见，节点列表保持独立滚动。
  PR: include
  image:
  ![Bound multiple proxy nodes](./assets/group-settings-hard-bound-multiple-nodes.png)

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: viewport
  sensitive_exclusion: N/A
  submission_gate: pending-owner-approval-for-push
  story_id_or_title: Account Pool/Components/Upstream Account Group Settings Dialog/Hard Bound Multiple Nodes
  state: request-trend-tooltip-details
  evidence_note: 验证分组绑定节点右侧的 24 小时请求图会在悬浮时显示时间桶、Success、Failure 与 Total requests 详情，且 tooltip 与参考界面复用同一套 inline chart surface。
  PR: include
  image:
  ![Bound proxy node request trend tooltip](./assets/group-settings-chart-tooltip.png)

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  sensitive_exclusion: N/A
  submission_gate: pending-owner-approval-for-push
  story_id_or_title: Account Pool/Pages/Upstream Account Create/Batch OAuth/Ready
  state: existing-group-settings-from-create-page
  evidence_note: 验证创建页里的真实分组设置入口已经和列表页使用同一套组件与数据契约，选中现有分组后会展示 `Direct`、协议标签和绑定节点趋势，不再泄露原始订阅地址。
  PR: include
  image:
  ![Batch OAuth ready group settings with bindings](./assets/batch-oauth-ready-group-settings-with-bindings.png)

## 变更记录

- 2026-03-26: 创建 spec，冻结 `/v1/*` 新语义、分组绑定 forward proxy 的运行时规则、接口契约与视觉证据目标。
- 2026-03-27: 视觉证据完成主人确认，spec 状态切换为已完成，并标记 PR 可复用截图。
- 2026-03-27: 增补线上 follow-up：分组绑定弹窗改为协议标签展示 + 独立滚动布局，并在分组绑定路径恢复显式 `Direct` 选项；后续补齐桌面宽度约束、刷新 Storybook 证据并等待截图提交授权。
