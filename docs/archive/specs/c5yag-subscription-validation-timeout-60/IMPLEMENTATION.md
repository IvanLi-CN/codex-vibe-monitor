# Implementation

## Current State

- `proxyUrl` 验证保持 5 秒单次探测预算。
- `subscriptionUrl` 验证保留 60 秒前端/后端整体预算用于拉取和解析订阅。
- 订阅解析后的 supported endpoints 进入全量扫描，不再只检查前 3 个。
- 节点扫描使用 10 个节点级并发槽位；每个节点最多顺序探测 3 次，每次探测预算 10 秒。
- 任意节点探测到 reachable status 后立即返回成功，并通过 cancellation token 通知其他在途探测尽快停止与清理。

## Validation

- `cargo test validate_subscription_candidate -- --nocapture`
