# 全项目 TTFT 口径实现状态（#6qe6u）

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖与 rollout 事实。

## Current Status

- Implementation: 已实现，进入远端 CI 与 PR 收敛
- Lifecycle: active
- Catalog note: TTFT canonical contract

## Coverage / rollout summary

- HTTP SSE、Responses Compact、Chat Completions 与 WebSocket turn 已复用同一首个非空模型输出 delta 识别规则。
- invocation、archive、分钟/小时 read model、live snapshot、账号/模型统计与 timeseries 已接入 nullable `first_token_ms` 及其样本聚合。
- owner-facing Dashboard、账号卡、统计、记录和调用详情已切换到 `firstToken*`；调用记录主信息并列展示 `TTFT` 与 `tUpstreamStreamMs` 对应的响应耗时，网络诊断保留独立的 `TTFB / 上游首字节`。
- 旧数据保持 `null`，旧 `firstResponseByteTotal*` 仅兼容读取且不参与 TTFT 聚合或 UI fallback。

## Remaining Gaps

- 本地实现与视觉验证已完成；视觉证据已获提交授权，交付步骤包含远端 CI、review 与 PR 合并。

## Related Changes

- Topic branch: `th/ttft-metrics`

## References

- `./SPEC.md`
- `./HISTORY.md`
