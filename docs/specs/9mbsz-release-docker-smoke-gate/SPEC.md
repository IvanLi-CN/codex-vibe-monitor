# Release 前 Docker Smoke Gate（#9mbsz）

## 状态

- Status: 待实现
- Created: 2026-03-01
- Last: 2026-03-01

## 背景 / 问题陈述

- 当前 CI 在 `main` push 时会构建并推送 GHCR 镜像、打 git tag、创建 GitHub Release，但不验证镜像是否能启动以及服务是否可用。
- 这会把“镜像可构建但不可运行”的问题延后到发版后才暴露，排查与回滚成本更高。

## 目标 / 非目标

### Goals

- 在 release_enabled=true 的发版路径中，把镜像 tag push 之前增加运行态 smoke gate：
  - 先 build 到 runner 本地（`load`）
  - `docker run` 启动容器
  - `GET /health` 返回 `ok`
- smoke 未通过时：阻断后续镜像 tag push、git tag 与 GitHub Release。

### Non-goals

- 不做 post-release 的“拉取已推送镜像再验证”的二次工作流。
- 不在 PR 阶段运行 docker smoke gate（如需可另开 spec）。
- 不扩展更严格的运行态验证范围（本次仅 `/health`）。

## 范围（Scope）

### In scope

- 修改 `.github/workflows/ci.yml` 的 release job 步骤：build(load) -> smoke -> push tags -> tag -> release。
- 新增 `.github/scripts/smoke-test-image.sh` 脚本（只做 `/health` 校验）。

### Out of scope

- 应用代码、HTTP API、Dockerfile 与部署文档内容变更。

## 需求（Requirements）

### MUST

- `release_enabled=true` 时，smoke 失败必须使 job 失败，并且不 push 镜像 tags、不打 tag、不建 Release。
- smoke 通过后，push 镜像 tags + git tag + GitHub Release 与原行为一致。

### SHOULD

- smoke 失败输出包含容器日志（便于定位启动失败原因）。

### COULD

- 后续扩展 smoke 覆盖 `/api/version` 或首页 200（本次不做）。

## 接口契约（Interfaces & Contracts）

None

## 验收标准（Acceptance Criteria）

- Given `release_enabled=true`
  When smoke test 超时或 `/health` 非 `ok`
  Then workflow 失败，且不会执行镜像 tag push / git tag / GitHub Release。
- Given `release_enabled=true`
  When smoke test 通过
  Then 镜像 tags 推送成功，且后续 git tag 与 GitHub Release 步骤执行成功。
- Given `release_enabled=false`（`type:docs` 或 `type:skip`）
  When workflow 在 `main` push 触发
  Then build/smoke/push/tag/release 相关步骤不会执行（与现有行为一致）。

## 非功能性验收 / 质量门槛（Quality Gates）

### Quality checks

- `bash -n .github/scripts/smoke-test-image.sh` 通过。

## 实现里程碑（Milestones / Delivery checklist）

- [ ] M1: Add spec + index row
- [ ] M2: Add smoke script
- [ ] M3: Wire release job to gate push on smoke success

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：runner 端口冲突导致 false negative（默认使用 18080；可用环境变量覆盖）。
- 假设：`ubuntu-latest` runner 具备 `docker` 与 `curl`。

## 参考（References）

- `.github/workflows/ci.yml`（docker job）
- `src/main.rs`：`GET /health` 返回 `ok`
