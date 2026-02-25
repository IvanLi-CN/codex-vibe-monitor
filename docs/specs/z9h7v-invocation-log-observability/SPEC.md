# 请求日志可观测性增强（IP / Cache Tokens / 分阶段耗时 / Prompt Cache Key）（#z9h7v）

## 状态

- Status: 已完成
- Created: 2026-02-25
- Last: 2026-02-25

## 背景 / 问题陈述

- 当前 `/api/invocations` 虽已包含 token 与成本，但缺少请求方来源信息（IP）、稳定请求标识（prompt cache key）与易读的阶段耗时展示。
- Web 端表格仅突出错误详情，无法在单次请求维度快速定位“慢在何处”。
- 代理链路已有 `payload` 与分阶段耗时字段落库能力，尚未系统化对外输出与前端展示。

## 目标 / 非目标

### Goals

- 在不变更 SQLite 表结构的前提下，补齐请求级上下文字段：`requesterIp`、`promptCacheKey`、`endpoint`、`failureKind`。
- `/api/invocations` 向前端稳定返回分阶段耗时字段，支持“首字节 / 总耗时”与完整阶段详情展示。
- Live 与 Dashboard 共用表格统一升级，主表保持简洁，详情区保留完整诊断信息。

### Non-goals

- 不新增独立请求详情页。
- 不改现有统计聚合口径与时序聚合逻辑。
- 不新增数据库列或迁移脚本。

## 范围（Scope）

### In scope

- `src/main.rs` 代理采集增强、`/api/invocations` 列表字段扩展。
- `src/main.rs` 启动阶段全量回填历史 proxy 记录中的 `payload.promptCacheKey`。
- `web/src/lib/api.ts` 类型对齐后端返回。
- `web/src/components/InvocationTable.tsx` 新增 cache/latency 列与通用详情展开区。
- `web/src/i18n/translations.ts` 新增中英文文案键。

### Out of scope

- 任何数据库 schema 变更。
- 采集敏感头（如 `Authorization`）或原文脱敏策略重构。
- 统计页图表结构改造。

## 需求（Requirements）

### MUST

- requester IP 提取优先级固定：`x-forwarded-for` 首值 > `x-real-ip` > `Forwarded(for=...)` > peer ip 兜底。
- prompt cache key 提取优先级固定：请求体候选 JSON 指针（`/prompt_cache_key`、`/promptCacheKey`、`/metadata/prompt_cache_key`、`/metadata/promptCacheKey`）> 请求头候选键（`x-prompt-cache-key` 等）。
- `build_proxy_payload_summary` 在成功/失败路径都包含 `requesterIp` 与 `promptCacheKey`（缺失时为 null）。
- `/api/invocations` 返回新增字段：`requesterIp`、`promptCacheKey`、`endpoint`、`failureKind`。
- 前端主表新增 `Cache Tokens` 与 `Latency` 列（`First byte / Total`），详情区展示完整阶段耗时。
- 启动回填会将历史记录中的 `payload.codexSessionId` 移除，并写入 `payload.promptCacheKey`。

### SHOULD

- 历史/非代理记录缺字段时前端统一展示 `—`，不抛错。
- 不影响 SSE 通道协议与统计接口行为，仅将字段名由 `codexSessionId` 变更为 `promptCacheKey`。

### COULD

- 后续按需增加“导出详情”或独立详情页。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 请求进入 `/v1/chat/completions` 或 `/v1/responses` 采集路径时，后端提取 IP 与 prompt cache key，并随 payload 一并落库。
- `/api/invocations` 通过 `json_extract(payload, ...)` 投影扩展字段，不依赖新增列。
- 前端表格默认显示关键字段（token/cost/latency），用户展开后看到请求元信息与阶段耗时明细。
- 启动阶段执行历史回填：读取 `request_raw_path` 指向的原始请求 JSON，提取 `prompt_cache_key` 后写回 payload。

### Edge cases / errors

- 若 `x-forwarded-for` 首值不可解析，则回退到下一级来源，不中断请求。
- 若 prompt cache key 候选键全部未命中，返回 `null` 并在前端显示 `—`。
- 若阶段耗时缺失（旧记录），前端逐项显示 `—`。

## 接口契约（Interfaces & Contracts）

### `GET /api/invocations` 记录对象（新增可选字段）

- `requesterIp?: string`
- `promptCacheKey?: string`
- `endpoint?: string`
- `failureKind?: string`

### `GET /api/invocations` 记录对象（已存在并沿用）

- `tReqReadMs?`、`tReqParseMs?`、`tUpstreamConnectMs?`、`tUpstreamTtfbMs?`、`tUpstreamStreamMs?`、`tRespParseMs?`、`tPersistMs?`、`tTotalMs?`

## 验收标准（Acceptance Criteria）

- Given 请求携带 `x-forwarded-for` 与 `metadata.prompt_cache_key`，When 请求完成并查询 `/api/invocations`，Then 返回 `requesterIp`、`promptCacheKey`、`cacheInputTokens` 与阶段耗时字段。
- Given 请求无转发头且 body 无 prompt_cache_key，When 请求完成，Then 前端详情对应字段显示 `—` 且页面无错误。
- Given 成功或失败记录，When 用户展开表格详情，Then 可见 endpoint、failureKind 与完整阶段耗时。
- Given 旧记录或 `source=xy` 记录缺扩展字段，When 页面渲染，Then 不崩溃且缺值显示 `—`。
- Given 历史 proxy 记录存在 `request_raw_path` 且 payload 缺 `promptCacheKey`，When 服务启动完成，Then 字段被自动回填且不会重复更新已完成记录。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo test`
- `cargo check`
- `cd web && npm run build`

### Manual verification

- 启动 backend/frontend 后打开 `/dashboard` 与 `/#/live`，验证新增列与详情展开可用。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: specs-first 建档与索引更新。
- [x] M2: 后端采集与接口输出增强（IP/prompt cache key/payload 投影）。
- [x] M3: 前端表格与文案升级（主表 + 详情）。
- [x] M4: 历史记录全量回填与幂等校验。
- [x] M5: 回归验证通过并完成本地提交。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：上游请求不保证稳定携带 `prompt_cache_key`，仍可能出现正常缺失。
- 开放问题：是否后续在 SQLite 增加独立 `prompt_cache_key` 列（本次不做）。
- 假设：现有代理链路 payload 存储可承载新增上下文字段。

## 变更记录（Change log）

- 2026-02-25: 初始化规格，冻结实现边界与验收口径。
- 2026-02-25: 完成后端字段采集、`/api/invocations` 投影扩展与前端表格升级，并通过 `cargo test`、`cargo check`、`web npm run build` 验证。
- 2026-02-25: 修复 SSE `records` 广播回查 SQL 投影不全问题，确保 `endpoint/requesterIp/promptCacheKey/failureKind` 与 `/api/invocations` 一致，并补充回归测试。
- 2026-02-25: 将对外字段从 `codexSessionId` 切换为 `promptCacheKey`，新增启动期历史数据全量回填与旧键清理，并补充回填幂等/异常分支测试。
