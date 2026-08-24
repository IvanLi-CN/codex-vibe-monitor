# Codex Vibe Monitor Context

This context freezes the invocation-observability terms that the product uses in diagnostics, badges, and companion docs. It exists to keep transport paths, request semantics, and response outcomes from drifting into overloaded labels.

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
