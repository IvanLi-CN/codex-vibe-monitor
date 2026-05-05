# 号池模块第一阶段：上游账号管理 - Implementation

## Current State

- Canonical spec: `docs/specs/g4ek6-account-pool-upstream-accounts/SPEC.md`
- Implementation summary: 已实现

## Migrated Implementation Notes

## 状态

- Status: 已实现
- Created: 2026-03-11
- Last: 2026-04-01

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust tests：schema 创建 / 迁移、加密 round-trip、OAuth 登录会话生命周期、callback state/TTL/single-use 校验、refresh 分类、usage payload 归一化、母号唯一性与 session 落库，以及“维护不阻塞无关账号启停 / 同账号写操作与维护严格串行 / 重复维护请求去重”并发回归。
- Web tests：账号列表与详情渲染、OAuth 轮询流程、API Key 表单、母号皇冠互斥、系统通知撤销、空态/错误态、导航路由。
- Browser smoke：本地打开 `号池 -> 上游账号`，验证新增 OAuth/API Key、同步与详情图表渲染。

## 文档更新（Docs to Update）

- `README.md`：新增账号管理 env、OAuth 配置与使用说明。
- `docs/deployment.md`：新增加密密钥、OAuth callback 与运维保活说明。
- `docs/specs/README.md`：状态与 PR 记录同步。
