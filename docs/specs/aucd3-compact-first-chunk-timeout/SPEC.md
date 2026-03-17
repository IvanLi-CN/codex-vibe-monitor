# 号池 compact 首 chunk 超时口径对齐（#aucd3）

## 状态

- Status: 部分完成（2/3）
- Created: 2026-03-17
- Last: 2026-03-17

## 背景 / 问题陈述

- `v1.12.6` 已修复号池 live upstream 复用整请求总超时 client 导致的约 `60s` 截流问题，但 101 线上 `/v1/responses/compact` 仍然明显偏慢。
- 线上日志与数据库样本显示，compact 请求经常在“等待第一个 upstream chunk”阶段被判失败，随后触发既有的同账号重试 / 跨账号 failover，导致总耗时膨胀到数分钟。
- 当前 `send_pool_request_with_failover` 已接收 capture-target 感知的 `handshake_timeout`，但真正等待首个 body chunk 时仍误用通用 `request_timeout`，没有对齐 compact 专用的 `openai_proxy_compact_handshake_timeout`。
- 这不是新增号池重放需求缺失，而是现有实现对 compact 首 chunk 等待预算使用错误。

## 目标 / 非目标

### Goals

- 让号池 `/v1/responses/compact` 在首个 upstream chunk 到达前，使用 compact 专用超时预算，而不是通用 `request_timeout`。
- 保持现有号池失败切换、429 重试、sticky key 与 replay 语义不变。
- 增加自动化回归，覆盖 compact 慢首 chunk 成功，以及普通 `/v1/responses` 仍按默认预算超时的对照场景。

### Non-goals

- 不新增号池路径对上游错误的请求重放或恢复能力。
- 不修改外部 relay、账号池路由策略或线上网络拓扑。
- 不改动 `/v1/responses`、`/v1/models` 等非 compact 路径现有超时语义。

## 范围（Scope）

### In scope

- `src/main.rs` 中 `send_pool_request_with_failover` 的首 chunk 超时口径。
- `src/tests/mod.rs` 中 capture-target / pool compact 慢首 chunk 回归测试。
- `docs/specs/README.md` 与当前 spec 的状态同步。

### Out of scope

- mid-stream gzip 损坏这类已经开始下发后的上游流损坏治理。
- 新的失败分类、熔断策略或数据库 schema 变更。

## 需求（Requirements）

### MUST

- 当请求目标是 `/v1/responses/compact` 时，号池在“首个 upstream chunk 到达前”的等待预算必须与 compact handshake timeout 对齐。
- 当请求目标不是 compact 时，号池首 chunk 等待预算必须继续使用现有 `request_timeout` 行为。
- 修复不得改变已有的账号选择、同账号重试、跨账号 failover 或 sticky/replay 流程。
- `request_timeout=200ms` 且 compact 专用 timeout=`400ms` 时，`/v1/responses/compact?mode=slow-first-chunk` 必须成功返回完整 JSON。
- 同样配置下，普通 `/v1/responses?mode=slow-first-chunk` 仍必须按现有语义报 `first upstream chunk` 超时。

### SHOULD

- 超时帮助函数命名应直接表达“pool 首 chunk 预算”含义，避免未来再次把 compact 路径落回通用预算。
- 回归测试应直接走 `proxy_openai_v1` + pool routing API key，贴近真实线上路径。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- `send_pool_request_with_failover` 进入发送循环前，基于 `original_uri + method` 推导本次请求的首 chunk 超时预算。
- 若 capture target 是 `ResponsesCompact`，首 chunk 预算使用调用方传入的 compact-aware `handshake_timeout`。
- 其他请求继续使用 `config.request_timeout` 作为首 chunk 预算。
- 现有 `read_pool_upstream_first_chunk_with_timeout`、失败记录与 failover 分支不改语义，只替换传入预算。

### Edge cases / errors

- 若 compact 在 headers 前超出 budget，仍走现有 handshake timeout 失败路径。
- 若 compact 已拿到首 chunk，后续中途断流仍按现有 `upstream_stream_error` 语义处理；本次不尝试补救已损坏响应。
- 若普通 `/v1/responses` 或其他非 compact 路径首 chunk 超时，仍保持当前失败行为。

## 验收标准（Acceptance Criteria）

- Given `request_timeout=200ms` 且 `openai_proxy_compact_handshake_timeout=400ms`，When 号池 POST `/v1/responses/compact?mode=slow-first-chunk`，Then 返回 `200 OK` 且 body 为完整 compact JSON。
- Given 相同配置，When 号池 POST `/v1/responses?mode=slow-first-chunk`，Then 返回 `502` 且错误体包含 `first upstream chunk`。
- Given 现有 compact handshake 回归，When 重新运行测试，Then 仍然通过，说明未破坏既有 compact 超时语义。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo test pool_openai_v1_responses_`
- `cargo test proxy_capture_target_compact_uses_dedicated_handshake_timeout`

### Quality checks

- `cargo fmt --all`
- `cargo test`

## 文档更新（Docs to Update）

- `docs/specs/README.md`
- `docs/specs/aucd3-compact-first-chunk-timeout/SPEC.md`

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 对齐号池 compact 首 chunk 等待预算。
- [x] M2: 补充 compact 成功 / 非 compact 对照回归测试。
- [ ] M3: 完成 fast-track 交付（提交、PR、merge、release、deploy、verify）。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：若把对齐逻辑做得过宽，会意外扩大非 compact 路径的首 chunk 等待时间。
- 风险：即便首 chunk 超时口径修复，`NSNGC` 仍可能存在独立的 mid-stream gzip 损坏；这属于后续单独问题，不在本次修复范围。
- 假设：当前 compact“非常慢”的主体开销来自首 chunk 误判失败后的既有 failover 链路，而不是最终成功那一轮本身极慢。

## 变更记录（Change log）

- 2026-03-17: 创建 spec，冻结 compact 首 chunk 超时错配的根因、边界与验收标准。
- 2026-03-17: 完成本地实现与 targeted regression；待 PR / release / deploy 收口。
