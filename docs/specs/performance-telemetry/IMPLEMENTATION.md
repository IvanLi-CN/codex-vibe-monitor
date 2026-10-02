# 外部性能观测实现

## Current coverage

锁定设计正在替换旧性能 SQLite 实现。旧实现事实及其验收卡保留为历史输入，不能证明新方案已完成或上线。所属架构为 ADR 0025，完整边界见设计与 METRICS。

## Deployment boundary

应用 PR 交付埋点、入口、provisioning、Agent 工具、归档和恢复合同。独立运维任务拥有 101 外部平台部署与生产切换；真实 public URL、权限、容量和 perf 条件需在上线时验证。

## Validation

新候选版本必须通过仓库 Rust/Web 检查、受控 Linux Compose、迁移、HTTPS 鉴权、CPU attach 和默认观测 A/B。证据绑定当前 Candidate，旧经验性验收卡不再适用于当前实现。
