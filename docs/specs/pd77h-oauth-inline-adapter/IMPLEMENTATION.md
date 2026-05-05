# OAuth 数据面内联合并 - Implementation

## Current State

- Canonical spec: `docs/specs/pd77h-oauth-inline-adapter/SPEC.md`
- Implementation summary: 待实现

## Migrated Implementation Notes

## 状态

- Status: 待实现
- Created: 2026-03-16
- Last: 2026-03-16

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: OAuth adapter 请求改写、模型列表归一化、SSE completed/error 提取、错误摘要。
- Integration tests: pool OAuth route 的 invalid_grant / token invalidated / 一次 stale token 恢复 / 单服务路径。
- E2E tests (if applicable): 线上等价流程的账号详情错误口径与重新授权后的路由表现。

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新 spec 入索引，旧 sidecar spec 改为重新设计
- `docs/specs/u8j4n-fixed-oauth-bridge-sidecar/SPEC.md`: 标记被本 spec 取代
- `README.md`: 删除 sidecar 双服务说明，改成单服务 OAuth 数据面说明
- `docs/deployment.md`: 删除 sidecar 部署与排障步骤，改成主进程内联语义

## Migrated Implementation Sections

## 实现前置条件（Definition of Ready / Preconditions）

- 目标/非目标、单进程边界与“无过渡兼容”策略已锁定
- `/v1/*` 对外兼容范围已确认
- pool 内部传输抽象已明确，不再要求实现阶段自行决定响应类型

### Quality checks

- `cargo check`
- `cargo test`
- `cd web && bun run test`
- `cd web && bun run build`
- 单镜像容器 smoke（只启动 `codex-vibe-monitor`）

## 计划资产（Plan assets）

- Directory: `docs/specs/pd77h-oauth-inline-adapter/assets/`
- In-plan references: `![...](./assets/<file>.png)`
- PR visual evidence source: maintain `## Visual Evidence` in this spec when PR screenshots are needed.

## Visual Evidence

## 资产晋升（Asset promotion）

None

## 实现里程碑（Milestones / Delivery checklist）

- [ ] M1: 规格与部署口径切换到单进程 OAuth adapter
- [ ] M2: 后端内联 OAuth adapter + pool 响应抽象完成
- [ ] M3: Web 文案与测试迁移到新口径
- [ ] M4: 快车道验证、PR、checks、review-loop 收敛完成
