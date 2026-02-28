# 在 Docker 镜像内内置 Xray（修复订阅节点验证失败）（#eg7zb）

## 状态

- Status: 待实现
- Created: 2026-03-01
- Last: 2026-03-01

## 背景 / 问题陈述

- 设置页添加订阅链接并点击“验证可用性”时，服务端会尝试用 Xray 启动本地 socks 路由来探测订阅节点。
- 目前发布镜像不自带 `xray`，导致验证失败，UI 报错类似：`failed to start xray binary ...` / `xray: no entry passed validation`。
- 影响：订阅节点无法在 UI 内完成验证与导入，forward proxy 功能不可用或体验明显受损。

## 目标 / 非目标

### Goals

- 发布镜像 **自带可执行的** `xray`（默认放置于 `/usr/local/bin/xray`），无需宿主机额外安装。
- CI release smoke 在 push 镜像前验证：镜像内可执行 `xray version`。
- 订阅验证失败时，若 Xray 启动/运行异常，错误消息可携带 Xray stderr 尾部内容，便于 UI 直接展示根因。

### Non-goals

- 引入 Xray geodata（geoip/geosite）或额外规则文件打包（当前 forward proxy 探测不依赖）。
- 改变 forward proxy 订阅解析与节点选择算法。

## 范围（Scope）

### In scope

- `/Dockerfile`：新增 Xray 下载/校验/拷贝的 build stage（GitHub Release + sha256 校验）。
- `/.github/workflows/ci.yml`：增加 smoke gate：`docker run ... xray version`（在 push 前执行）。
- `/src/main.rs`：Xray 启动失败时记录并回传 stderr 片段（有长度上限），并补充单元测试覆盖。
- `/docs/deployment.md`：补充镜像已内置 Xray 与相关 env 变量说明。

### Out of scope

- 新增/修改任何外部 API（仅增强错误消息与镜像内容）。

## 需求（Requirements）

### MUST

- runtime 镜像内存在 `xray` 且可执行（默认路径 `/usr/local/bin/xray`）。
- Docker build 时使用 XTLS/Xray-core GitHub Releases 资产：
  - `Xray-linux-*.zip` + 对应 `.dgst` 的 sha256 校验。
- CI release smoke 在 push 前验证 `xray version` 成功（exit code = 0）。
- 订阅验证不再因为“缺少/不可执行 xray”而失败。

### SHOULD

- 诊断信息：当 Xray 进程启动失败或提前退出时，错误信息包含 stderr 尾部（例如最后 4KB），并做长度上限保护。

## 接口契约（Interfaces & Contracts）

None

## 验收标准（Acceptance Criteria）

- Given 使用默认发布镜像
  When 执行 `docker run --rm <image> xray version`
  Then 退出码为 0 且输出包含版本信息。

- Given 使用默认发布镜像
  When 启动容器并访问 `GET /health`
  Then 30s 内返回 `ok`。

- Given 在设置页添加订阅链接
  When 点击“验证可用性”
  Then 不应出现“failed to start xray binary / xray not found”类错误（允许因节点本身不可用而失败）。

- Given 故意输入一个会导致 Xray 启动失败的节点（或让 Xray 进程立即退出）
  When 验证该节点/订阅
  Then UI/接口返回的错误信息包含 Xray stderr 片段，便于定位根因。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: `cargo test --locked --all-features`

### Quality checks

- Formatting: `cargo fmt --all -- --check`

## 实现里程碑（Milestones / Delivery checklist）

- [ ] M1: Dockerfile 打包 Xray（Release 资产 + sha256 校验）并拷贝到 runtime 镜像
- [ ] M2: CI smoke gate 增加 `xray version` 检查并阻断坏镜像发布
- [ ] M3: 后端记录 Xray stderr 并在验证错误中回传，补单测覆盖
- [ ] M4: 更新 `docs/deployment.md` 的部署说明（Xray 内置 + env）

## 方案概述（Approach, high-level）

- 使用 Docker multi-stage build 在构建阶段下载指定版本 Xray-core release 资产，并通过 `.dgst` 校验 sha256，保证可复现与供应链完整性。
- runtime 镜像仅携带 `xray` 二进制（以及 LICENSE），避免引入不必要依赖。
- 后端把 Xray stderr 写入 runtime_dir 下的文件，并在失败时截断读取尾部返回，提高可诊断性且避免 UI 被超长日志污染。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：GitHub Releases 资产命名若变更会导致 build 失败（通过明确报错与 CI smoke 早发现）。
- 假设：当前 forward proxy 探测流程不依赖 geodata 文件（geoip/geosite）。
