# 号池流式上游误用整请求超时（#ynr8z）

## 状态

- Status: 进行中
- Created: 2026-03-17
- Last: 2026-03-17

## 背景 / 问题陈述

- `#154` 已经把代理编码协商改为透明透传，但 101 线上在 `2026-03-17 10:00 +08:00` 之后仍持续出现 `routeMode=pool` 的 `upstream_stream_error`。
- 线上失败样本的 `t_total_ms` 大量集中在约 `60024ms`，与部署环境的 `REQUEST_TIMEOUT_SECS=60` 精确对齐，说明失败不是随机网络抖动，而是服务自身在大约 `60s` 处把长流式响应截断。
- 当前实现中，号池上游请求复用了带 `.timeout(config.request_timeout)` 的 shared `reqwest::Client`；对于 `/v1/responses*` 这类先快速返回首字节、随后长时间流式输出的请求，这个整请求总超时会在 body 传输阶段把流切断。
- `gzip unexpected end of file` 只是截断后的诊断结果：服务先把流中途切断，随后原始响应落盘与 usage 解析才看到半截 gzip；根因仍然是我们自己的客户端超时策略，而不是新增号池重放逻辑缺失。

## 目标 / 非目标

### Goals

- 让号池上游的流式请求不再复用整请求总超时 client，避免在 body 传输阶段被 `REQUEST_TIMEOUT_SECS` 硬切断。
- 保持现有握手超时、请求体读取超时、429 重试与号池请求重放语义不变。
- 补齐自动化回归，覆盖“短 `request_timeout` 配置下，号池流式响应仍可完整透传”的场景。

### Non-goals

- 不新增“号池路径对上游错误自动重放恢复服务”的新能力。
- 不修改外部 relay、NSNGC、Traefik、Caddy 或部署网络拓扑。
- 不调整非号池后台轮询任务对 `request_timeout` 的既有依赖。

## 范围（Scope）

### In scope

- `src/main.rs` 中 `HttpClients` 的 client 分工，以及号池上游发送路径对 client 的选择。
- `src/tests/mod.rs` 中覆盖号池流式透传的后端回归测试。
- `docs/specs/README.md` 与当前 spec 的状态同步。

### Out of scope

- 上游账号路由策略、熔断策略或重试策略设计变更。
- 新的失败分类口径与数据库 schema 改动。

## 需求（Requirements）

### MUST

- 号池上游发送路径不得继续复用带整请求总超时的 shared `reqwest::Client`。
- 号池 API key 与 OAuth 的 live upstream 请求都必须使用“无整请求总超时”的 client。
- 非号池后台轮询/抓取路径继续保留 shared client 的 `request_timeout` 行为，不得把后台任务的超时保护一并移除。
- 号池路径已有的 `handshake_timeout` 包装、`request_read_timeout`、429/5xx/401/403 分支与 sticky/replay 语义必须保持不变。
- 当 `config.request_timeout=200ms` 且上游首字节后延迟 `400ms` 再返回下一块数据时，号池 `/v1/slow-stream` 仍必须成功返回完整 body，而不是在约 `200ms` 后报 `upstream_stream_error`。
- 当 `config.request_timeout=200ms` 且号池 `/v1/responses` 返回慢速成功 SSE 时，代理仍必须完整返回 `response.completed` 事件。

### SHOULD

- client 命名与职责应明确区分“后台抓取总超时”与“号池流式上游无总超时”，避免未来再次误用。
- 至少保留一条直接映射线上现象的 `/v1/responses` 回归测试，避免只验证通用 `/v1/slow-stream` 而漏掉真实路径。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- `HttpClients::build` 继续构建：
  - 带整请求总超时的 shared client，用于后台轮询与非流式抓取；
  - 不带整请求总超时的号池上游 client，用于 pool live upstream；
  - 现有 forward proxy client 继续保留原职责。
- `send_pool_request_with_failover`、号池 OAuth live upstream、号池 API key live upstream 都改为使用号池专用 client。
- 号池 live upstream 仍然只在握手阶段受 `handshake_timeout` 保护；一旦首字节已到达，后续 body 传输不得再被 `request_timeout` 截断。

### Edge cases / errors

- 若号池上游在首字节前超过握手预算，仍按既有 `upstream_handshake_timeout` / `failed_contact_upstream` 逻辑返回。
- 若请求体读取阶段超时，仍按现有 `request_body_read_timeout` 路径失败；本次修复不能改变上传读体语义。
- 若后台轮询或普通抓取超过 `request_timeout`，仍应保持既有失败行为；本次修复不能把 shared client 的保护全局移除。

## 验收标准（Acceptance Criteria）

- Given `config.request_timeout=200ms`，When 号池 GET `/v1/slow-stream` 命中上游 `chunk-a` 后等待 `400ms` 才收到 `chunk-b`，Then 响应仍为 `200 OK` 且 body 为 `chunk-achunk-b`。
- Given 同样的 `config.request_timeout=200ms`，When 号池 POST `/v1/responses?mode=slow-success` 命中慢速成功 SSE，Then 响应仍为 `200 OK` 且包含 `response.completed`。
- Given 同样的配置，When 非号池后台任务仍使用 shared client，Then 其 `request_timeout` 行为不变。
- Given 线上样本全文检索 `routeMode=pool` 的 `upstream_stream_error`，When 修复发布并部署后复核，Then 不应再出现稳定集中在约 `60000ms` 的同类截断模式。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo test pool_openai_v1_e2e_stream_survives_short_request_timeout`
- `cargo test pool_openai_v1_responses_stream_survives_short_request_timeout`
- `cargo test proxy_openai_v1_e2e_stream_survives_short_request_timeout`

### Quality checks

- `cargo fmt --check`
- `cargo test`

## 文档更新（Docs to Update）

- `docs/specs/README.md`：新增规格索引，并在流程推进后同步状态。
- `docs/specs/ynr8z-pool-stream-total-timeout/SPEC.md`：记录实现进展、验证与 PR/发布状态。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 为号池上游引入无整请求总超时 client，并只在 pool 发送路径上使用。
- [x] M2: 增加 `/v1/slow-stream` 与 `/v1/responses` 的短超时慢流成功回归测试。
- [ ] M3: 完成 fast-track 交付（提交、push、PR、checks、review-loop、merge、release、deploy、verify）。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：若误把 shared client 的超时保护全部移除，后台轮询可能失去既有超时收敛；本次必须严格限定变更范围。
- 风险：若直接复用 forward proxy client，可能意外改变 redirect/connect 语义；实现应尽量保持 pool 既有行为，只去掉整请求总超时。
- 假设：线上稳定落在约 `60s` 的 `upstream_stream_error` 峰值主要由 shared client 的 `.timeout(config.request_timeout)` 触发，修复后应显著消失。

## 变更记录（Change log）

- 2026-03-17: 创建 spec，冻结根因、边界与回归标准。
- 2026-03-17: 完成 `HttpClients` 分工修复；号池 live upstream 改走无整请求总超时 client，并为“首 chunk 超时 / OAuth 非流式读体超时”补回显式预算，`cargo test` 全量 395 项通过。
