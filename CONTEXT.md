# Codex Vibe Monitor Context

This context freezes the project-specific terms used in invocation observability and release automation. It exists to keep transport paths, request semantics, response outcomes, and publication surfaces from drifting into overloaded labels.

## Invocation Compaction

**Compact**:
The legacy dedicated compaction request path backed by `/v1/responses/compact`.
_Avoid_: 远程压缩V1, remote compaction v1, `/v1/responses` compaction

**远程压缩V2**:
Server-side compaction semantics that run inside `/v1/responses` without changing the transport endpoint.
_Avoid_: Compact, `/v1/responses/compact`, normal Responses badge

**压缩请求**:
The request-side declaration that a `/v1/responses` call enabled remote compaction V2 semantics.
_Avoid_: 压缩响应, 最终已压缩

**压缩响应**:
The response-side proof that the upstream actually emitted a compaction item for the invocation.
_Avoid_: 压缩请求, 已启用压缩

## Runtime Read Models

**Summary Projection**:
An exact, immutable in-memory `StatsResponse` read model for a validated Summary
selection. It is hydrated and reconciled outside the HTTP request path; it never
stands for an approximate aggregate or a request-time SQLite/file fallback.
_Avoid_: partial summary, zero fallback, request-time summary rebuild

**Last-Good Snapshot**:
The most recent exact read-model value that remains internally retained while a
background refresh is unavailable. Its retention does not permit a stale or
partial value to be represented as a fresh projection.
_Avoid_: fabricated empty summary, implicit fresh cache

**Canonical Invocation Classification**:
The revisioned durable outcome fact for one terminal invocation: its failure
class and actionable state. It is written once by terminal persistence or by a
controlled compatibility materializer and is the only classification source for
Summary, rollup and aggregate readers.
_Avoid_: payload-derived reader classification, window-specific failure class

**Classification Coverage**:
Durable proof that a live or archived record range has current Canonical
Invocation Classification. A reader with incomplete coverage is exact-unavailable
rather than allowed to infer a result from raw payload bytes.
_Avoid_: implicit legacy fallback, partial exactness

**Archive Classification Overlay**:
The durable identity-keyed Canonical Invocation Classification for a record in an
immutable archive. It avoids rewriting archive files while making their outcome
available to a background projector or rollup builder.
_Avoid_: mutable archive, request-time archive repair

**SQLite Pressure Defer**:
A deliberate refusal of low-priority background database work before it acquires
SQLite because the shared pressure gate is closed. It leaves durable progress
unchanged and has one in-memory scheduler eligibility deadline plus an
event/deadline wake; it is not a failed database operation.
_Avoid_: lock retry, millisecond polling, work-completed audit

**SQLite Lock Failure**:
An actual SQLite `BUSY` or `LOCKED` result after work attempts database access.
It follows the operation's bounded error backoff and is distinct from a pressure
defer.
_Avoid_: pressure defer, successful no-op

## Dashboard 上游账号活动

**统计卡片**:
Dashboard「上游账号」视图中呈现单个账号聚合指标的卡片，包含 TTFT、请求数、成本与 Token 等，每张卡片都有一个主值。
_Avoid_: 指标块, KPI 卡, 小卡片

**紧凑主值**:
统计卡片中按可用宽度自适应呈现的主数值文本；可从完整值切换为带量级的短文本，但不改变原始聚合值。
_Avoid_: 截断值, 省略值

**指标语义量级**:
紧凑主值按指标类型使用的单位体系：计数和 Token 使用 K/M/B/T，成本使用 $K/$M/$B/$T，耗时使用 ms/s/min/h。
_Avoid_: 通用单位, K 秒

**精度预算**:
紧凑主值默认保留三位有效数字；可用宽度不足时逐级降低精度，并在舍入进位跨越量级时同步升级单位。
_Avoid_: 固定小数位, 1000K

**常驻指标值**:
统计卡片在卡面默认可见的主值与明细值；二者均服从紧凑主值合同，标识符、时间戳与百分比不属于此类。
_Avoid_: Tooltip 明细, 原始数据

**可用内容宽度**:
统计卡片扣除内边距、图标及间距后，数值文本实际可占用的单行宽度；它不是固定视口或卡片断点。
_Avoid_: 窄卡片阈值, 固定像素宽度

## Release Automation

**Release 正文**:
GitHub Release 页面中面向使用者的发布说明；只包含明确的用户向 release notes，不混入流程元数据。
_Avoid_: PR 元数据, 发布审计, CI 诊断

**自动生成发布说明**:
基于相邻发布 tag 之间变更生成的用户向 Release 正文；它是当前 Release 正文的来源。
_Avoid_: PR 正文转抄, 流程元数据拼接

**发布决策快照**:
与主线提交绑定的不可变自动发布决策记录，保存版本意图与发布计算所需字段；手工覆盖字段只存在于本次 workflow 的临时快照，不写入该记录。
_Avoid_: Release 正文, 手工覆盖审计, 公开变更说明

**PR 发布评论**:
附在源 PR 上的版本交付追溯记录；它独立于 GitHub Release 页面。
_Avoid_: Release 正文, 发布说明
