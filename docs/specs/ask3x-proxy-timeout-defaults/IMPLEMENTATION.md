# 反向代理默认超时口径统一为 60s / 180s - Implementation

## Current State

- Canonical spec: `docs/specs/ask3x-proxy-timeout-defaults/SPEC.md`
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-03-10
- Last: 2026-03-10

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust tests：覆盖配置默认值、环境变量覆盖、compact 专属等待超时命中、普通 responses 继续命中通用等待超时。
- 文档检查：全文检索 `OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS`、`OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS`、`OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS`，确认口径一致。

## 文档更新（Docs to Update）

- `docs/specs/README.md`：新增规格索引并在完成后写入 PR / checks 状态。
- `README.md`：更新默认值，并补充 compact 专属超时为可选覆盖说明。
- `docs/deployment.md`：改为默认值/可选覆盖的事实描述，移除推荐措辞。
- `docs/plan/fd4pw-proxy-request-read-timeout-rc-fix/PLAN.md`：清理遗留的冲突超时数字表述。
