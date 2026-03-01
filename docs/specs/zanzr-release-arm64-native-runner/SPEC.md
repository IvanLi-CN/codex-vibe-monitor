# Release 构建加速：arm64 迁移到 GitHub-hosted ARM runner（#zanzr）

## 状态

- Status: 部分完成（3/4）
- Created: 2026-03-01
- Last: 2026-03-01

## 背景 / 问题陈述

- 当前 release workflow 在 `linux/arm64` 平台构建时依赖 QEMU（x64 runner + setup-qemu）。
- 在实际运行中，`Build smoke image (linux/arm64, load)` 会在 QEMU 下编译 Rust 依赖，常见表现为长时间停留在 `Compiling ...`，导致 release wall time 大幅增加（可达 20+ 分钟）。
- 该瓶颈与业务代码无关，属于 CI 架构选择导致的性能问题。

## 目标 / 非目标

### Goals

- arm64 release 构建改用 GitHub-hosted 原生 ARM runner（`runs-on: ubuntu-24.04-arm`），移除 QEMU。
- release workflow 拆分为 meta + per-arch build/smoke/push candidate + publish（manifest/tag/release），保持当前 smoke gate 与 manifest 校验逻辑。
- 通过 `buildcache-amd64`/`buildcache-arm64` 分离缓存，减少跨架构污染并提升复用命中率。
- Dockerfile 的 Rust build 分层：先编译依赖层、再编译业务层，避免每次 release 都全量重编译依赖。

### Non-goals

- 不变更 Rust/前端业务逻辑、HTTP API、数据库 schema。
- 不新增额外平台（维持 `linux/amd64` + `linux/arm64`）。

## 范围（Scope）

### In scope

- `.github/workflows/ci.yml`：拆分 release job，arm64 job 使用 `ubuntu-24.04-arm`，保持 smoke gate 与发布逻辑。
- `Dockerfile`：Rust build 分层缓存。
- `.dockerignore`：减小 build context，避免 `.git`/产物导致缓存失效。

### Out of scope

- 更改 smoke gate 覆盖范围（仍以 `--help`、`xray version`、`/health` 为准）。

## 验收标准（Acceptance Criteria）

- Given `release_enabled=true` 的 main push 触发 release，
  When workflow 运行，
  Then arm64 构建 job 的 runner 为 `ubuntu-24.04-arm`，且流程中不再包含 QEMU 初始化步骤。

- Given release 构建完成，
  When 校验版本 tag manifest，
  Then 必须同时包含 `linux/amd64` 与 `linux/arm64`。

- Given 同等条件下的 release 运行，
  When 对比优化前后的 wall time，
  Then release 总耗时显著降低，且 arm64 构建不再出现“QEMU 下长时间 Rust 依赖编译”瓶颈。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 建立 spec 文档并登记索引
- [x] M2: release workflow job 拆分 + arm64 迁移到 `ubuntu-24.04-arm`
- [x] M3: Dockerfile Rust build 分层 + `.dockerignore`
- [ ] M4: 以 `type:patch + channel:rc` 跑通一次 release 验证（不更新 latest）

## 风险 / 备注

- ARM runner 可能存在排队；但相较于 QEMU 下长时间编译，整体稳定性与吞吐通常更好。
- Buildx registry cache 体积可能增加；通过按架构拆分 `buildcache-*` 降低互相污染与竞态。
