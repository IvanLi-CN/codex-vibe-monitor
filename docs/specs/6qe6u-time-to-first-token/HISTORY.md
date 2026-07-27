# 全项目 TTFT 口径演进历史（#6qe6u）

> 这里记录影响长期理解的关键演进；规范正文仍以 `./SPEC.md` 为准。

## Decision Trace

- 2026-07-25：建立 canonical TTFT 合同，停止以 HTTP TTFB 或请求阶段累计耗时冒充模型首 Token 时间。
- 2026-07-25：参考 `sub2api@43d4bae` 的产品定义，同时统一其 HTTP/WS 实现分叉，以首个非空模型输出 delta 为唯一终点。
- 2026-07-25：完成 HTTP/WS 采集、future-only 持久化、分钟/小时聚合、API 与 owner-facing UI 迁移；旧累计首字节字段退出 TTFT 展示与聚合。
- 2026-07-26：调用记录主信息收敛为 `TTFT + 响应耗时`；响应耗时仅取 `tUpstreamStreamMs`，端到端总耗时降级为阶段诊断。
- 2026-07-26：调用记录网络摘要同样收敛为 TTFT 与响应耗时的平均值和 P95；旧 TTFB/总耗时摘要字段仅保留兼容读取。

## Key Reasons / Replacements

- 旧 `firstResponseByteTotal*` 表示 `request read + parse + connect + HTTP first byte`，只适合网络阶段诊断，不能回答模型何时开始输出。
- 历史数据缺少事件到达时间证据，因此保持 `null` 比推算一个看似精确但错误的 TTFT 更可靠。

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
