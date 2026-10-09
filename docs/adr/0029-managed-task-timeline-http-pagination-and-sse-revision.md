# Task Timeline HTTP Pages and SSE Revision Notifications

Status: Superseded by ADR 0030

时间线区间通过固定水位的 HTTP keyset 页传输，按 `(startedAt, segmentId)` 续页，并用 `afterRevision` 读取修订；使用 `windowHours=12` 时，服务端先读取水位、再选择窗口，避免分页期间的行修订或浏览器时钟偏差造成历史遗漏。每个增量遍历都复用已提交基线的 `from` / `to`，避免滚动窗口在后续修订读取中漂移。SSE `/v2` 只通知维护库水位与 `observedAt`，客户端暂存 HTTP 页并在完整后提交，再追平分页及重连期间的修订。本决定 supersedes ADR 0026 中通过 SSE 事件传输时间线区间的交付选择，仅替换时间线传输边界，不改变持续采集、持久化、保留或观测缺口语义。
