# GHCR 发布切换多架构 manifest（amd64 + arm64）（#wtwsn）

## 状态

- Status: 已完成（5/5）
- Created: 2026-03-01
- Last: 2026-03-01

## 背景 / 问题陈述

- 线上发布的 `stable/latest` 标签此前是单架构 manifest（`linux/amd64`），部分平台展示为 `arch 未知`。
- 当前 release job 采用单次 `buildx load` + `docker push`，无法生成包含多平台条目的 manifest list。
- 需要在不改变业务代码的前提下，将发布产物升级为可识别的双架构镜像，并保留发布前 smoke 门禁。

## 目标 / 非目标

### Goals

- 让 `stable` 发布同时产出 `linux/amd64` 与 `linux/arm64` 的 manifest list。
- 发布前分别对 amd64/arm64 进行完整 smoke（`--help`、`xray version`、`/health`）。
- 推送后强制校验标签 manifest 必须同时包含上述两个平台，缺失即阻断后续 tag/release。

### Non-goals

- 不变更 Rust/前端业务逻辑、HTTP API 或数据库 schema。
- 不新增 `arm/v7`、`s390x` 等额外平台。
- 不调整现有 release intent（`type:*` + `channel:*`）规则。

## 范围（Scope）

### In scope

- `.github/workflows/ci.yml`：release job 多架构构建、分平台 smoke、推送后 manifest 校验。
- `.github/scripts/smoke-test-image.sh`：新增 `SMOKE_PLATFORM` 支持，在指定平台执行 smoke。
- `README.md`：补充 GHCR 多架构发布与校验行为说明。

### Out of scope

- 部署编排（Compose/K8s）改造。
- 运行时参数或环境变量语义变更。

## 接口契约（Interfaces & Contracts）

- `smoke-test-image.sh`：
  - 入参保持不变：`smoke-test-image.sh <image-tag>`。
  - 新增可选环境变量：`SMOKE_PLATFORM`（例如 `linux/arm64`），用于 `docker run --platform`。
- Release workflow：
  - `PLATFORMS` 固定为 `linux/amd64,linux/arm64`。
  - 镜像推送改为 `buildx build --platform <multi> --push`，产物为 manifest list。

## 验收标准（Acceptance Criteria）

- Given `release_enabled=true`，When release job 运行，Then amd64 与 arm64 smoke 都通过后才允许推送正式 tags。
- Given 多架构推送完成，When 校验版本 tag manifest，Then 必须同时检测到 `linux/amd64` 与 `linux/arm64`。
- Given 任一 smoke 或 manifest 校验失败，When workflow 执行，Then job fail 且不执行 git tag/GitHub Release。
- Given `type:docs`/`type:skip`，When workflow 触发，Then release 路径仍保持跳过。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 建立 specs 文档并登记索引
- [x] M2: smoke 脚本支持 `SMOKE_PLATFORM`
- [x] M3: release job 拆分 amd64/arm64 smoke gate
- [x] M4: release job 改为多架构 push，并新增 manifest 平台校验
- [x] M5: README 更新多架构发布说明并完成本地校验

## 风险 / 备注

- arm64 smoke 依赖 QEMU，执行时间可能波动，默认将 arm64 smoke timeout 提升到 120 秒。
- manifest 展示在部分平台可能存在缓存延迟；以 registry 实际 manifest 校验结果为准。
- 发布流程会写入 run 级 `candidate-*` 中间标签用于组装最终 manifest；当前未内建自动回收，后续可按仓库保留策略补充清理。
