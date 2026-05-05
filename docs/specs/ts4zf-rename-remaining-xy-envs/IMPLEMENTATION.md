# 修正剩余 `XY_*` 环境变量命名 - Implementation

## Current State

- Canonical spec: `docs/specs/ts4zf-rename-remaining-xy-envs/SPEC.md`
- Implementation summary: 已完成

## Migrated Implementation Notes

## 状态

- Status: 已完成
- Created: 2026-03-10
- Last: 2026-03-10
- Supersedes: `docs/specs/2uaxk-remove-xyai-legacy-ingest/SPEC.md` 中“仍然通用的 `XY_*` 配置键不重命名”的旧非目标；该 supersede 仅限公开环境变量命名，不恢复任何 XYAI 采集逻辑。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo test`
- 针对 config parsing 的定向回归：新 canonical 名称可成功解析；被移除的旧键会 fail-fast。
- 文档/代码残留扫描：针对 README、Dockerfile、部署文档与源码中的公开 env 名称执行检索确认。

## 文档更新（Docs to Update）

- `docs/specs/README.md`：新增本 spec，并在交付完成后写入 PR / checks / review-loop 状态。
- `README.md`：把公开 env 示例、retention/archive 说明与 migration note 统一到新命名。
- `docs/deployment.md`：把运维配置说明与 archive 路径描述统一到新命名。
- `Dockerfile`：把 runtime 默认 env 与 Xray 相关注释更新到新命名。
