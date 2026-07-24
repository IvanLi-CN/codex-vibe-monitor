# Worktree bootstrap 与显式依赖初始化（#v7se4）

> 当前有效规范以本文为准；实现覆盖与当前状态见 `./IMPLEMENTATION.md`，关键演进原因见 `./HISTORY.md`。

## 背景 / 问题陈述

- linked worktree 需要在首次进入和后续 checkout 后稳定继承本地配置与依赖，但依赖恢复失败不应破坏 Git checkout。
- 当前仓库已经有 shared Git hooks、copy-missing-only `.env.local` 同步与真实 linked worktree smoke；本规范将“自动依赖恢复”和“手动完整 bootstrap”作为同一套可复用实现固定下来。
- archived `docs/archive/specs/v7se4-worktree-bootstrap/SPEC.md` 是历史来源；本文是当前 canonical spec。

## 目标 / 非目标

### Goals

- 保持 `post-checkout` bootstrap 安全、可重复执行，并只在 linked worktree 中恢复依赖。
- 提供统一的依赖恢复实现，覆盖 repo root、`web`、`docs-site` 的 Bun 依赖和 Rust crate 缓存。
- 用 smoke test 锁住自动/手动入口、主/linked worktree 区分、锁定参数与失败隔离行为。

### Non-goals

- 不复制 `node_modules`、SQLite DB、`.codex/xray-forward` 或其他运行态目录。
- 不修改 HTTP API、SSE、数据库 schema 或前端业务逻辑。
- 不自动安装 Bun、Cargo、系统库或 Playwright 浏览器等外部前置条件。

## 范围（Scope）

### In scope

- repo-local CLI：`bun run hooks:install`、`bun run worktree:bootstrap`、`bun run worktree:setup`。
- shared Git hook wrapper、`scripts/worktree-sync.paths`、worktree bootstrap/setup smoke test。
- README / AGENTS 中面向维护者的 bootstrap 与 setup 说明。

### Out of scope

- 本地 secret 内容、依赖版本升级、CI required check 名称调整。
- 自动修复缺失系统依赖或开发机 Bun 安装。

## 需求（Requirements）

### MUST

- linked worktree 的 `post-checkout` 自动路径必须尝试执行三项 `bun install --frozen-lockfile` 和一项 `cargo fetch --locked`；主 worktree 的 `post-checkout` 不得安装依赖。
- `worktree:bootstrap` 必须继续遵守 copy-missing-only；目标文件已存在时不得覆盖。
- `worktree:setup` 必须按四项依赖任务执行，使用 locked 参数；单项失败后必须继续其余任务并汇总失败。
- 自动 hook 必须返回成功并告警；手动 `worktree:bootstrap` 在存在失败时必须返回非零。
- smoke test 必须使用 fake Bun/Cargo 验证上述调用链，且不得真实联网安装依赖。

### SHOULD

- setup 脚本保持薄封装，优先复用 Bun、Cargo 与现有 lockfiles。
- 文档应明确主 worktree 与 linked worktree 的触发差异，以及自动/手动失败码语义。

### COULD

- 后续可在 setup 中增加轻量健康检查，但必须保持显式触发。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 新建或切换 linked worktree 时，shared `post-checkout` hook 调用当前 checkout 的 bootstrap runner；runner 同步 manifest 中缺失的本地资源，并调用依赖 setup。
- 依赖 setup 在 repo root、`web`、`docs-site` 运行 `bun install --frozen-lockfile`，随后在 repo root 运行 `cargo fetch --locked`。
- 主 worktree 的 `post-checkout` 只同步本地资源，不运行依赖 setup。
- 自动 hook 忽略依赖 setup 的最终失败码并打印补救提示；手动 `bun run worktree:bootstrap` 保留失败码。

### Edge cases / errors

- 当前 worktree 已存在 `.env.local` 时，bootstrap 必须跳过且不覆盖。
- 目标依赖目录不存在或依赖状态需要更新时，由对应 locked install 命令负责恢复。
- 若当前 revision 缺少 bootstrap 脚本，shared hook wrapper 继续安全 no-op。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name）                 | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes）                     |
| ---------------------------- | ------------ | ------------- | -------------- | ------------------------ | --------------- | ------------------- | --------------------------------- |
| `bun run worktree:setup`     | CLI          | internal      | Modify         | None                     | repo tooling    | contributors        | 执行三项 Bun + 一项 Rust 依赖恢复 |
| `bun run worktree:bootstrap` | CLI          | internal      | Modify         | None                     | repo tooling    | contributors        | 同步资源并聚合依赖恢复失败        |
| `post-checkout` bootstrap    | Git hook     | internal      | Modify         | None                     | repo tooling    | linked worktrees    | 自动恢复依赖但不阻断 checkout     |

### 契约文档（按 Kind 拆分）

- None

## 验收标准（Acceptance Criteria）

- Given 新 linked worktree 缺失 `.env.local` 和依赖目录
  When shared `post-checkout` hook 触发
  Then worktree 获得缺失 `.env.local`，并执行三项 frozen Bun install 与一次 locked Cargo fetch。

- Given 主 worktree 触发 `post-checkout`
  When setup 脚本运行
  Then 不执行任何 Bun 或 Cargo 依赖命令。

- Given 任一 Bun/Cargo 依赖任务失败
  When 自动 hook 继续执行
  Then 其余任务仍执行、输出失败摘要且 hook 返回 0；手动 bootstrap 返回非零。

- Given CI 运行 `scripts/test-worktree-bootstrap.sh`
  When fake `bun` 和 fake `cargo` 捕获 setup 调用链
  Then 测试不联网且能证明命令参数、执行顺序和失败隔离。

## 验收清单（Acceptance checklist）

- [x] linked 自动依赖恢复与主 worktree 跳过已由 smoke test 覆盖。
- [x] locked Bun/Cargo 安装路径与失败隔离已由 fake `bun`/`cargo` 覆盖。
- [x] README / AGENTS 已区分 bootstrap 与 setup。
- [x] archived spec 的历史语义已迁移到 canonical spec。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `bash scripts/test-worktree-bootstrap.sh`
- `cargo check`
- `cd web && bun run test`

### UI / Storybook (if applicable)

- Not applicable.

### Quality checks

- 不新增 required check 名称。
- 不改变 release label gate 或 GitHub branch protection 语义。

## Visual Evidence

None

## Related PRs

- None

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：把依赖安装放进 linked checkout hook 会增加网络和耗时；本规范固定为逐项 best-effort，失败不阻断 checkout。
- 假设：Bun 是仓库唯一 JS package manager，且 root、`web/`、`docs-site/` 都由 Bun 管理。

## 参考（References）

- `docs/archive/specs/v7se4-worktree-bootstrap/SPEC.md`
- `README.md`
- `AGENTS.md`
- `scripts/test-worktree-bootstrap.sh`
