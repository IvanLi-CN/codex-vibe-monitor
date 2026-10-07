# Task Timeline HTTP Pages and SSE Revision Notifications

Status: Accepted

任务时间线通过固定水位的 HTTP 游标页传输区间数据，使用 `afterRevision` 分页读取变更；SSE `/v2` 仅通知维护库修订水位和观测时间。这样快照大小不受单个事件的容量上限约束，重连与分页期间的并发写入仍可由修订水位追平；代价是客户端需要暂存分页结果并在完整后原子提交。本决定明确 supersedes ADR 0026 中“通过 SSE 事件传输时间线区间”的交付选择，仅替换该数据传输边界，不改变持续采集、持久化、保留或观测缺口边界。
