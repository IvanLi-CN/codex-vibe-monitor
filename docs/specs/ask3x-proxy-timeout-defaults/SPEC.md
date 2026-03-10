# 反向代理默认超时口径统一为 60s / 180s（#ask3x）

## 状态

- Status: 部分完成（4/5）
- Created: 2026-03-10
- Last: 2026-03-10

## 背景 / 问题陈述

- 当前仓库对反向代理上游等待超时存在多套互相冲突的口径：代码默认值仍为 `45s`，README 与部署文档写成 `300s`，历史排障文档又留下 `120s` 说明，导致实现、配置说明与排障信息无法相互印证。
- `/v1/responses/compact` 的语义与普通代理请求不同，但当前实现没有 compact 专属默认超时，导致“远程压缩”与普通路径复用同一等待上游响应预算。
- `OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS` 的默认口径需要继续保持 `180s`，本次不能把“请求体读取总超时”和“等待上游响应超时”混成一套配置含义。
- 部署文档当前带有“建议显式配置”“建议起始值”“可降到”等措辞，会把默认值误写成推荐配置；本次需要改成纯事实描述，不再暗示必须或推荐设置这些环境变量。

## 目标 / 非目标

### Goals

- 将非 compact 代理路径的默认上游等待超时统一为 `60s`。
- 为 `/v1/responses/compact` 增加专属默认上游等待超时 `180s`，并支持独立环境变量覆盖。
- 保持 `OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS` 默认值为 `180s`，且职责继续限定为“请求体读取总超时”。
- 清理 README、部署文档与历史排障文档中的错误或冲突表述，使仓库内检索结果只剩当前唯一口径。

### Non-goals

- 不调整代理请求读体失败分型、状态码映射或持久化 schema。
- 不修改 compact 识别规则、Fast mode rewrite、usage 注入或 pricing 逻辑。
- 不把 compact 专属超时变成必配环境变量；未配置时必须依赖服务默认值工作。

## 需求（Requirements）

### MUST

- `OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS` 继续作为通用代理上游等待超时变量存在，默认值改为 `60` 秒。
- 新增 `OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS`，仅用于 `/v1/responses/compact` 的上游等待超时覆盖；默认值为 `180` 秒。
- `/v1/responses`、`/v1/chat/completions`、`/v1/models` 与其它非 compact 通用透传路由都必须使用 `60s` 默认上游等待超时。
- `/v1/responses/compact` 必须使用 `180s` 默认上游等待超时，且不影响其它路径。
- `OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS` 默认值必须为 `180` 秒，并继续仅用于请求体读取阶段。
- README、`docs/deployment.md`、`docs/plan/fd4pw-proxy-request-read-timeout-rc-fix/PLAN.md` 中不得保留 `45/120/300` 这些与当前默认口径冲突的超时说法。
- 部署文档不得再使用“建议设置”“建议起始值”“可降到”等措辞描述这些代理超时变量。

### SHOULD

- 超时分流逻辑应内聚到 `ProxyCaptureTarget` 或统一 helper，避免在多个请求路径重复手写 compact 分支。
- 回归测试应覆盖默认值解析、环境变量覆盖与 compact/非 compact 路径的实际命中差异。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 启动配置解析时：
  - `OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS` 未设置时取 `60s`；
  - `OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS` 未设置时取 `180s`；
  - `OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS` 未设置时取 `180s`。
- 普通代理透传路径与 `/v1/models` 上游请求继续使用通用超时配置。
- capture target 路径在发送上游请求前，根据 `ProxyCaptureTarget` 选择等待超时；仅 `ResponsesCompact` 命中 compact 专属值。
- 握手超时后的错误行为保持现状：返回 `502`，并沿用现有 `upstream_handshake_timeout` / `failed_contact_upstream` 分型与记录逻辑。

### Edge cases / errors

- 若 compact 专属环境变量缺失、非法或为非正数，必须回退到默认值 `180s`。
- 若通用环境变量缺失、非法或为非正数，必须回退到默认值 `60s`。
- 若请求体读取超时发生，仍由 `OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS` 控制，不能被 compact 专属等待超时覆盖。

## 验收标准（Acceptance Criteria）

- Given 未设置相关环境变量，When 服务启动解析配置，Then 通用代理上游等待超时为 `60s`、compact 专属等待超时为 `180s`、请求体读取总超时为 `180s`。
- Given 设置 `OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS=61` 与 `OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS=181`，When 服务启动解析配置，Then 两者分别按对应值生效。
- Given 通用超时较短且 compact 专属超时更长，When `/v1/responses/compact` 命中延迟上游，Then compact 请求仍可成功；Given 同配置下 `/v1/responses` 命中同类延迟上游，Then 普通 responses 请求按 `upstream_handshake_timeout` 返回 `502`。
- Given 通用非 compact 代理路由命中慢上游，When 超过 `60s` 默认预算，Then 行为仍归类为既有 `upstream_handshake_timeout`，不引入新的错误口径。
- Given README、部署文档与历史计划全文检索代理超时变量，When 检查结果，Then 不再出现 `45/120/300` 的冲突默认值说法，且部署文档只保留“默认值/可选覆盖”的事实描述。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust tests：覆盖配置默认值、环境变量覆盖、compact 专属等待超时命中、普通 responses 继续命中通用等待超时。
- 文档检查：全文检索 `OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS`、`OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS`、`OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS`，确认口径一致。

### Quality checks

- `cargo fmt --check`
- `cargo test`

## 文档更新（Docs to Update）

- `docs/specs/README.md`：新增规格索引并在完成后写入 PR / checks 状态。
- `README.md`：更新默认值，并补充 compact 专属超时为可选覆盖说明。
- `docs/deployment.md`：改为默认值/可选覆盖的事实描述，移除推荐措辞。
- `docs/plan/fd4pw-proxy-request-read-timeout-rc-fix/PLAN.md`：清理遗留的冲突超时数字表述。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 配置层新增 compact 专属上游等待超时，并将通用默认值改为 `60s`、请求体读取默认值改为 `180s`。
- [x] M2: 代理路径按 compact/非 compact 分流等待超时，既有超时失败分型保持不变。
- [x] M3: Rust 回归覆盖默认值、override 与 compact/非 compact 的超时命中差异。
- [x] M4: README、部署文档与历史排障计划的超时口径完成统一清理。
- [ ] M5: fast-track 交付完成（提交、push、PR、checks、review-loop、spec 状态同步）。

## 风险 / 假设

- 风险：若线上部署已显式覆盖旧的 `OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS`，代码默认值变更不会自动改变该实例行为；需要依赖部署端自行决定是否保留覆盖。
- 风险：历史 `docs/plan/**` 属于兼容保留目录，本次只清理会造成当前默认值歧义的条目，不做整批迁移或删除。
- 假设：`/v1/responses/compact` 继续保持当前精确路径匹配，不引入额外 compact 变体路由。

## 参考（References）

- `docs/specs/g3amk-codex-remote-compact-observability/SPEC.md`
- `docs/plan/fd4pw-proxy-request-read-timeout-rc-fix/PLAN.md`
