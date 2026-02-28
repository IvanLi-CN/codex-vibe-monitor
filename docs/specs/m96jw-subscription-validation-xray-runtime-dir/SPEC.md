# 修复订阅验证路径下 xray 运行目录缺失导致添加失败（#m96jw）

## 状态

- Status: 进行中（2/3）
- Created: 2026-03-01
- Last: 2026-03-01

## 背景 / 问题陈述

- 设置页“添加订阅链接”在订阅包含 `vless/vmess/trojan/ss` share link 时，点击“验证可用性”后快速失败，无法添加订阅。
- 后端报错为：`failed to write xray config: .codex/xray-forward/...json`。
- 根因是候选项验证路径会直接走 `XraySupervisor::spawn_instance`，而该路径未保证 `xray_runtime_dir` 已存在。

## 目标 / 非目标

### Goals

- 修复订阅候选校验路径，确保写 xray 配置前一定创建运行目录。
- 添加回归测试，防止未来回归到“目录不存在导致写配置失败”。
- 保持既有 API 与校验语义不变。

### Non-goals

- 不修改前端弹窗交互与按钮状态机。
- 不修改订阅解析、节点探测策略与超时策略。

## 范围（Scope）

### In scope

- `src/main.rs`：`XraySupervisor::spawn_instance` 在写配置前创建 `runtime_dir`。
- `src/main.rs` tests：新增针对验证路径目录创建的回归测试。

### Out of scope

- Docker 镜像内 xray 二进制安装策略。
- 代理探测目标与探测轮次策略。

## 验收标准（Acceptance Criteria）

- 订阅校验路径不再出现 `failed to write xray config: .codex/xray-forward/...` 类错误。
- 新增回归测试可稳定通过，且断言不会回归到“写配置失败”。
- 相关 Rust 测试通过。

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 复现失败并定位到 `spawn_instance` 写配置前未创建目录
- [x] M2: 修复目录创建逻辑 + 新增回归测试
- [ ] M3: fast-flow 下完成 PR + checks + review-loop 收敛
