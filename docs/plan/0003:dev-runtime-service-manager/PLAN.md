# 开发环境 devctl+zellij 保活（#0003）

## 状态

- Status: 待实现
- Created: 2026-02-20
- Last: 2026-02-20

## 问题陈述

当前仓库的长驻开发服务（backend / frontend）主要通过 `nohup + logs/*.pid` 启动与管理：

- PID 文件可能漂移/陈旧（尤其当服务被其它方式启动或重启后），导致 stop/status 难以可信。
- 日志、状态、停止方式缺少统一入口，不符合 Codex 环境的最新长驻服务方案。

需要对齐到 `devctl + zellij`：一个 service 一个 zellij session，日志落到 `.codex/logs/*.log`，并通过 `devctl status/logs/down` 统一管理。

## 目标 / 非目标

### Goals

- 以 `~/.codex/bin/devctl` + zellij 作为唯一的长驻开发服务启动方式（No fallback）。
- 统一启动/停止/日志/状态命令与脚本入口。
- 文档口径对齐：`AGENTS.md` 为强约束，`README.md` 提供推荐方式并保留通用 quickstart。

### Non-goals

- 不改变运行端口与参数：backend `127.0.0.1:8080`，frontend `127.0.0.1:60080`。
- 不引入 pm2/systemd 等其它进程管理器。
- 不清理历史日志文件。

## 范围（Scope）

### In scope

- `scripts/` 中的 dev server 启动脚本迁移为 `devctl`。
- 新增 stop/status 辅助脚本。
- `.gitignore` 忽略 `.codex/` 目录。
- 更新 `AGENTS.md` 与 `README.md` 的开发运行口径。

### Out of scope

- 后端/前端业务逻辑修改。
- CI、Docker、发布流程修改。

## 验收标准（Acceptance Criteria）

- Given 在仓库根目录
  When 执行 `./scripts/start-backend.sh`
  Then `~/.codex/bin/devctl --root <repo> status backend` 为 running，且 `curl http://127.0.0.1:8080/health` 返回 `ok`。
- Given 在仓库根目录
  When 执行 `./scripts/start-frontend.sh`
  Then `~/.codex/bin/devctl --root <repo> status frontend` 为 running，且 `curl http://127.0.0.1:60080/` 可达。
- Given backend/frontend 已 running
  When 再次执行 `./scripts/start-*.sh`
  Then 脚本返回 0 并提示已在运行（不应因端口占用报错）。
- When 执行 `./scripts/stop-backend.sh` 与 `./scripts/stop-frontend.sh`
  Then 对应 `devctl status` 变为 stopped，端口不再监听。
- `.codex/` 不应出现在 `git status` 未跟踪文件中（已被 `.gitignore` 忽略）。

## 非功能性验收 / 质量门槛（Quality Gates）

- 本地至少执行 1 条与改动相关的自动化验证：
  - `bash -n scripts/start-backend.sh`
  - `bash -n scripts/start-frontend.sh`
  - `bash -n scripts/stop-backend.sh`
  - `bash -n scripts/stop-frontend.sh`

## 里程碑（Milestones）

- [ ] M1: `scripts/` 启动/停止脚本迁移为 `devctl`（No fallback）
- [ ] M2: 文档口径对齐（AGENTS.md + README.md + .gitignore）
- [ ] M3: 最小验证与 PR 交付（PR + checks 结果明确）

## 风险与开放问题（Risks / Open questions）

- 本仓库脚本强依赖 `zellij` + `~/.codex/bin/devctl`；若不满足则启动失败（符合 No fallback 的预期）。
