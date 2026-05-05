# 号池上游 413 原账号补试与切号（#ben7x）

## 背景 / 问题陈述

- 上游偶发返回 `413 Payload Too Large` 时，直接切号或直接透传会放大一次性误报。
- 经验上持续 `413` 更常见于 API Key 上游异常，但 pool 路由需要保持所有账号类型的一致处理。
- 本地请求体超限与上游返回 `413` 是不同故障面；前者仍应直接拒绝，后者需要进入 pool 的上游重试 / failover 语义。

## 目标 / 非目标

### Goals

- 任一 pool 上游账号首次返回 `413` 后，必须用原账号追加 1 次请求。
- 原账号补试仍为 `413` 时，停止同账号补试，并沿用现有 pool failover 候选选择、sticky 排除、distinct-account 上限和 timeout 预算切换账号。
- 若没有可切账号、候选耗尽、或最终失败仍是上游 `413`，调用方必须收到 `HTTP 413`。
- 上游 `413` 必须作为独立 failure kind 记录，避免与本地 `body_too_large` 混淆。

### Non-goals

- 不修改本地请求体大小限制与本地 `413` 返回路径。
- 不新增配置项、数据库 schema 或前端设置。
- 不改变既有 `429`、`5xx`、auth failure、timeout 的重试和 failover 语义。

## 功能与行为规格

- live-first pool 尝试收到上游 `413` 时，先记录本次 HTTP failure；请求体可 replay 后继续使用同一账号补试。
- replay / capture failover 主循环收到上游 `413` 时，只允许 `same_account_retry_index=2` 的一次补试；第二次仍为 `413` 后把该账号排除并进入下一个 distinct account。
- distinct-account 预算耗尽时，如果最后一个具体上游错误是 `413`，外部响应状态码保持 `413`，但 terminal attempt 仍可记录预算耗尽原因。
- `429` 仍保持立即切号或按组配置重试的既有语义；`5xx` 仍保持现有同账号重试预算。

## 接口契约

- 外部 HTTP 行为：pool `/v1/*` 上游最终 `413` 向调用方返回 `HTTP 413`，错误 JSON 壳保持现有格式。
- 内部 failure kind：新增 `upstream_http_413` 表示上游账号返回 `413`。
- attempts 行为：上游 `413` attempt 记录 `http_status=413`、`failure_kind=upstream_http_413`；同一账号最多出现两次连续 `413` attempt。

## 验收标准

- Given 单账号首次上游 `413`、第二次成功，Then 调用方收到成功响应，且该账号总共被请求 2 次。
- Given 首个账号连续两次上游 `413`、第二个账号成功，Then 调用方收到成功响应，且第二个账号被请求 1 次。
- Given 单账号连续两次上游 `413` 且无可切账号，Then 调用方收到 `HTTP 413`。
- Given 三个 distinct accounts 都连续两次上游 `413`，Then 调用方收到 `HTTP 413`，并保留 budget exhausted terminal attempt。
- Given 上游 `429` 或 `5xx`，Then 既有重试与切号语义不变。

## 非功能性验收 / 质量门槛

- `cargo fmt --check`
- `cargo check`
- Targeted Rust tests covering upstream `413` same-account retry, switch-account success, single-account final `413`, distinct-budget final `413`, existing `429`, and existing `5xx` behavior.
