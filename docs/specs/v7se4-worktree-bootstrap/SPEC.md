# Worktree bootstrap 与显式依赖初始化（#v7se4）

> 当前有效规范以本文为准；实现覆盖与当前状态见 `./IMPLEMENTATION.md`，关键演进原因见 `./HISTORY.md`。

## 背景 / 问题陈述

- linked worktree 需要稳定继承本地配置，但 checkout hook 不应承担联网安装依赖等重型动作。
- 当前仓库已经有 shared Git hooks、copy-missing-only `.env.local` 同步与真实 linked worktree smoke；本规范将“轻量 bootstrap”和“显式依赖 setup”作为长期边界固定下来。
- archived `docs/archive/specs/v7se4-worktree-bootstrap/SPEC.md` 是历史来源；本文是当前 canonical spec。

## 目标 / 非目标

### Goals

- 保持 `post-checkout` bootstrap 轻量、安全、幂等，只补缺失本地资源。
- 提供显式 `bun run worktree:setup`，一次安装 repo root、`web/` 与 `docs-site/` 的 Bun 依赖。
- 用 smoke test 锁住默认 bootstrap 不安装依赖、setup 才调用依赖安装的行为。

### Non-goals

- 不在 `post-checkout` 中默认执行 `bun install`。
- 不复制 `node_modules`、SQLite DB、`.codex/xray-forward` 或其他运行态目录。
- 不修改 HTTP API、SSE、数据库 schema 或前端业务逻辑。

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

- `post-checkout` 自动路径不得安装依赖、不得创建或复制 `node_modules`。
- `worktree:bootstrap` 必须继续遵守 copy-missing-only；目标文件已存在时不得覆盖。
- `worktree:setup` 必须由开发者显式执行，并按 repo root、`web/`、`docs-site/` 覆盖 Bun 依赖安装。
- smoke test 必须验证默认 bootstrap 的 no-deps 边界和 setup 的安装调用链，且不得真实联网安装依赖。

### SHOULD

- setup 脚本保持薄封装，优先复用 Bun 与现有 package manifests。
- 文档应明确 bootstrap 与 setup 的职责差异，避免维护者误以为 checkout 会自动安装依赖。

### COULD

- 后续可在 setup 中增加轻量健康检查，但必须保持显式触发。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 新 linked worktree checkout 时，shared `post-checkout` hook 调用当前 checkout 的 bootstrap runner；runner 只安装/维护 hook 链并同步 manifest 中缺失的本地资源。
- 开发者需要完整依赖环境时，在目标 worktree 执行 `bun run worktree:setup`；脚本依次在 repo root、`web/`、`docs-site/` 运行 `bun install`。

### Edge cases / errors

- 当前 worktree 已存在 `.env.local` 时，bootstrap 必须跳过且不覆盖。
- 目标依赖目录不存在时，bootstrap 不负责创建；setup 的 Bun install 负责正常依赖安装。
- 若当前 revision 缺少 bootstrap 脚本，shared hook wrapper 继续安全 no-op。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name）                 | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes）               |
| ---------------------------- | ------------ | ------------- | -------------- | ------------------------ | --------------- | ------------------- | --------------------------- |
| `bun run worktree:setup`     | CLI          | internal      | New            | None                     | repo tooling    | contributors        | 显式安装 root/web/docs 依赖 |
| `bun run worktree:bootstrap` | CLI          | internal      | Modify         | None                     | repo tooling    | contributors        | 明确保持 no-deps 轻量边界   |
| `post-checkout` bootstrap    | Git hook     | internal      | Modify         | None                     | repo tooling    | linked worktrees    | 自动路径不得安装依赖        |

### 契约文档（按 Kind 拆分）

- None

## 验收标准（Acceptance Criteria）

- Given 新 linked worktree 缺失 `.env.local`
  When shared `post-checkout` hook 触发
  Then worktree 获得缺失 `.env.local`，且不会创建 root、`web/`、`docs-site/` 的 `node_modules`。

- Given 开发者执行 `bun run worktree:setup`
  When setup 脚本运行
  Then repo root、`web/`、`docs-site/` 均执行一次 `bun install`。

- Given CI 运行 `scripts/test-worktree-bootstrap.sh`
  When fake `bun` 捕获 setup 调用链
  Then 测试不联网且能证明 setup 覆盖三个依赖安装位置。

## 验收清单（Acceptance checklist）

- [x] bootstrap no-deps 边界已由 smoke test 覆盖。
- [x] setup 显式安装路径已由 fake `bun` 覆盖。
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

- 风险：把依赖安装放进 checkout hook 会让 Git 操作依赖网络和耗时命令；本规范固定为显式 setup。
- 假设：Bun 是仓库唯一 JS package manager，且 root、`web/`、`docs-site/` 都由 Bun 管理。

## 参考（References）

- `docs/archive/specs/v7se4-worktree-bootstrap/SPEC.md`
- `README.md`
- `AGENTS.md`
- `scripts/test-worktree-bootstrap.sh`
