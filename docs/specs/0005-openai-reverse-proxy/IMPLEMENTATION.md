# OpenAI Reverse Proxy（`/v1/*` 透明透传） - Implementation

## Current State

- Canonical spec: `docs/specs/0005-openai-reverse-proxy/SPEC.md`
- Migrated from legacy source: `docs/plan/0005:openai-reverse-proxy/PLAN.md`
- Legacy source retention: pending delete approval
- Implementation summary: See companion notes and linked PR/check history for implementation context.

## Migrated Implementation Notes

## 测试策略

- 单元测试：URL 组装、hop-by-hop 头过滤。
- 集成测试（本地临时上游服务）：验证认证头透传、query 透传、状态/响应头透传、流式响应透传、上游不可达返回 502。
- 回归验证：`cargo check`、`cargo test`。

## 里程碑

- [x] M1 配置与路由接入（`OPENAI_UPSTREAM_BASE_URL` + `/v1/*`）
- [x] M2 代理请求/响应透传（含 header 过滤与流式响应）
- [x] M3 测试与文档更新（README + 自动化测试通过）
