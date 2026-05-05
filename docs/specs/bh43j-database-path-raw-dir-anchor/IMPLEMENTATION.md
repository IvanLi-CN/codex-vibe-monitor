# 数据库环境变量重命名与 raw 路径锚点修复 - Implementation

## Current State

- Canonical spec: `docs/specs/bh43j-database-path-raw-dir-anchor/SPEC.md`
- Implementation summary: 已完成（5/5）

## Migrated Implementation Notes

## 状态

- Status: 已完成（5/5）
- Created: 2026-03-09
- Last: 2026-03-09

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Rust tests：覆盖 `DATABASE_PATH` 生效、`XY_DATABASE_PATH` fail-fast、raw 目录锚定数据库父目录、cwd 兼容读取旧相对路径、orphan sweep 不再依赖 cwd。
- 构建验证：`cargo fmt --all -- --check`、`cargo test --locked --all-features`、`cargo check --locked --all-targets --all-features`。

## 文档更新（Docs to Update）

- `README.md`
- `docs/deployment.md`
- `docs/specs/README.md`
