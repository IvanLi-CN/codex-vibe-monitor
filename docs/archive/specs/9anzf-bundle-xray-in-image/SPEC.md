# Bundle Xray-core in Docker image for forward proxy subscriptions（#9anzf）

## 状态

- Status: 部分完成（2/3）
- Created: 2026-03-01
- Last: 2026-03-01

## 背景 / 问题陈述

- 当前 Docker 镜像运行时未内置 `xray`（Xray-core）二进制。
- 当用户在 UI 中添加“订阅链接”（Subscription URL）且订阅包含 `vmess/vless/trojan/ss` 等 share link 时，服务端会尝试启动 `xray` 生成本地 socks5 转发以完成可用性探测。
- 由于容器内缺少 `xray`，会导致订阅验证失败（典型报错：`failed to start xray binary: xray`），从而使该功能在容器部署场景不可用。

## 目标 / 非目标

### Goals

- Docker 运行时镜像内置可工作的 `xray`（Xray-core）二进制，确保订阅验证功能在容器内开箱即用。
- Release 流水线增加 smoke 覆盖：确保发布镜像中 `xray` 可执行且可被运行（最小检查）。
- 发布一个 stable patch，使 `latest` 对应镜像具备该能力。

### Non-goals

- 不新增/修改任何 HTTP API 与前端功能。
- 不引入 musl 静态编译或更换 runtime base。
- 不改变 forward proxy / xray 的运行逻辑（仅确保镜像内具备依赖与 smoke 覆盖）。

## 范围（Scope）

### In scope

- `/Dockerfile`: 增加 Xray-core 下载/安装 stage，并复制到 runtime image。
- `/.github/workflows/ci.yml`: release smoke 增加 `xray` presence check（并确保在 push 之前执行）。

### Out of scope

- 订阅解析/探测策略调整（例如探测更多条目、不同探测目标等）。

## 需求（Requirements）

### MUST

- 镜像内默认 `PATH` 可直接执行 `xray`（即 `/usr/local/bin/xray`）。
- CI release job 在 push 镜像 tag 之前验证：
  - `xray` 可执行（例如 `xray version` 或等效命令成功退出）。

### SHOULD

- 版本固定（pin）Xray-core 下载版本，避免外部依赖漂移导致不可重复构建。
  - 当前 pin: `v26.2.6`

## 接口契约（Interfaces & Contracts）

None

## 验收标准（Acceptance Criteria）

- Given 容器运行于默认配置
  When 在 UI 添加一个包含 share link 的订阅 URL 并点击“验证可用性”
  Then 不再因缺少 `xray` 导致验证直接失败（允许因线路不可达等真实原因失败）。

- Given release job 构建发布镜像
  When smoke 运行
  Then 在 push tag 之前 `xray` presence check 通过，否则阻断发布。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: Docker image bundles Xray-core binary (xray in PATH)
- [x] M2: Release workflow smoke checks xray before pushing tags
- [ ] M3: Stable patch release + ops verification

## 参考（References）

- Reported UI error: `subscription proxy probe failed: failed to start xray binary: xray; no entry passed validation`
