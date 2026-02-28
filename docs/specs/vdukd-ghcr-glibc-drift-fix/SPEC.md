# Fix GHCR image GLIBC drift (Debian bookworm runtime)（#vdukd）

## 状态

- Status: 部分完成（2/3）
- Created: 2026-03-01
- Last: 2026-03-01

## 背景 / 问题陈述

- `v0.11.0` 发布的镜像在 Debian 12 (bookworm, glibc 2.36) 运行时启动即退出：
  - `/lib/x86_64-linux-gnu/libc.so.6: version 'GLIBC_2.39' not found (required by codex-vibe-monitor)`
- 该现象表明：Rust 二进制在 glibc >= 2.39 的环境中构建/动态链接后，被拷入 bookworm 运行时镜像。
- 影响：服务容器起不来，Traefik 未加载 router，外部访问表现为 404。

## 目标 / 非目标

### Goals

- 让发布镜像可在 Debian 12 bookworm（glibc 2.36）正常运行（不再出现 `GLIBC_2.39 not found`）。
- 固定 build stage 与 runtime base 的 OS variant，避免 glibc 漂移。
- 在 CI release 流程加入镜像启动烟测门禁：smoke 失败则不 push 镜像 tags、不打 git tag、不创建 GitHub Release。
- 发布一个 stable patch（预期 `v0.11.1`）修复 `latest`。

### Non-goals

- 切换到 musl 静态编译（`x86_64-unknown-linux-musl`）。
- 升级/更换 runtime base（例如改为更新的 Debian/Ubuntu）。
- 调整 Traefik/部署配置。

## 范围（Scope）

### In scope

- `/Dockerfile`：将 Rust build stage pin 到 bookworm（与 runtime 对齐）。
- `/.github/workflows/ci.yml`：release job 改为 `build(load) -> smoke -> push`。

### Out of scope

- 任何后端 API / SSE / 前端逻辑变更。

## 需求（Requirements）

### MUST

- Docker build stage 使用 `rust:1.91.0-bookworm`（若 CI 验证不存在，再回退 `rust:1.91-bookworm`）。
- CI 在 push 前对 `:v${APP_EFFECTIVE_VERSION}` 做 smoke：
  - `docker run --rm <image>:v... codex-vibe-monitor --help`
  - 启动容器并探活 `GET /health` 返回 `ok`
- smoke 失败时：
  - `docker push` 不应执行
  - “Create and push git tag / Create GitHub Release” 不应执行

### SHOULD

- smoke 失败时输出可诊断信息：`docker ps` + `docker logs`。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- Release pipeline（stable）：
  - Build 镜像到本地 Docker daemon（`load: true`），打上 `:vX.Y.Z`（以及 `:latest`）。
  - 运行 smoke（`--help` + `/health`）。
  - 仅当 smoke 通过时 push tags。

### Edge cases / errors

- 若 `rust:1.91.0-bookworm` tag 不存在，改用 `rust:1.91-bookworm` 并在 PR 中说明原因。

## 接口契约（Interfaces & Contracts）

None

## 验收标准（Acceptance Criteria）

- 新发布镜像执行 `codex-vibe-monitor --help` 正常返回（exit code = 0），无 `GLIBC_2.39 not found`。
- 新发布镜像启动后 30s 内 `GET /health` 返回 `ok`。
- CI release job 的 smoke step 失败时，不会 push 镜像 tags，也不会创建 git tag / GitHub Release。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: `cargo test --locked --all-features`

### Quality checks

- Formatting: `cargo fmt --all -- --check`

## 文档更新（Docs to Update）

- None（本修复不改变使用方式；仅修复镜像构建与发布门禁）

## 资产晋升（Asset promotion）

None

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: Pin Rust build stage to Debian bookworm（避免 glibc 漂移）
- [x] M2: Add CI smoke gate before pushing image tags
- [ ] M3: Release stable patch and verify `:latest` is usable

## 方案概述（Approach, high-level）

- 通过固定 Rust builder 镜像的 OS variant（bookworm）让编译/链接环境与 runtime 对齐。
- 通过 CI “build -> smoke -> push” 把坏镜像阻断在发布前，而不是让 `latest` 漂到线上。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：`docker/build-push-action` 的 `load: true` 与 cache 配置在 release job 上可能需要微调（以 CI 结果为准）。
- 假设：服务在 `XY_LEGACY_POLL_ENABLED=false` 下无需外部环境变量即可启动并响应 `/health`。

## 参考（References）

- GitHub Release: v0.11.0
- Bad image digest: `ghcr.io/ivanli-cn/codex-vibe-monitor@sha256:4f8cf36367d6e7a5cba7496cee119bc205b06a118df54171c51a6dfa162ac327`
