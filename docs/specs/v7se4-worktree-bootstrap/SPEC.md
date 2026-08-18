# Worktree bootstrap 与显式依赖初始化（#v7se4）

> 当前有效规范以本文为准；实现覆盖与当前状态见 `./IMPLEMENTATION.md`，关键演进原因见 `./HISTORY.md`。

## 背景 / 问题陈述

- linked worktree 需要在首次进入和依赖输入失效时稳定继承本地配置与依赖，但普通 checkout 不应重复执行联网或重型恢复。
- 当前仓库已经有 Lefthook 生成的 shared Git hooks、copy-missing-only `.env.local` 同步与真实 linked worktree smoke；本规范将“按输入指纹恢复”和“手动完整 bootstrap”作为同一套可复用实现固定下来。
- archived `docs/archive/specs/v7se4-worktree-bootstrap/SPEC.md` 是历史来源；本文是当前 canonical spec。

## 目标 / 非目标

### Goals

- 保持 `post-checkout` bootstrap 安全、可重复执行，并让普通 linked checkout 在四个依赖 surface 都有效时不执行 Bun/Cargo。
- 提供统一的依赖恢复实现，覆盖 repo root、`web`、`docs-site` 的 Bun 依赖和 Rust crate 缓存。
- 用 smoke test 锁住自动/手动入口、主/linked worktree 区分、锁定参数与失败隔离行为。
- 将 repo 外全局 `lefthook` 2.1.7 或更高版本的可执行性作为 shared `post-checkout` hook 的启动前置条件，并在安装入口中显式失败；repo-local `node_modules/.bin/lefthook` 不得伪装该前置条件。

### Non-goals

- 不复制 `node_modules`、SQLite DB、`.codex/xray-forward` 或其他运行态目录。
- 不修改 HTTP API、SSE、数据库 schema 或前端业务逻辑。
- 不自动安装 Bun、Cargo、系统库或 Playwright 浏览器等外部前置条件。

## 范围（Scope）

### In scope

- repo-local CLI：`bun run hooks:install`、`bun run worktree:bootstrap`、`bun run worktree:setup`。
- Lefthook shared Git hooks、`scripts/worktree-sync.paths`、worktree bootstrap/setup smoke test。
- README / AGENTS 中面向维护者的 bootstrap 与 setup 说明。

### Out of scope

- 本地 secret 内容、依赖版本升级、CI required check 名称调整。
- 自动修复缺失系统依赖或开发机 Bun 安装。

## 需求（Requirements）

### MUST

- linked worktree 的 `post-checkout` 自动路径只在首次、对应依赖目录缺失或该 surface 输入指纹变化时执行对应的 `bun install --frozen-lockfile` 或 `cargo fetch --locked`；Bun `ok` 状态必须保留 manifest 的全部直接依赖包，Cargo `ok` 状态必须保留 `Cargo.lock` 的全部 registry archive。四项 surface 有效时不得执行 Bun/Cargo。主 worktree 的 `post-checkout` 不得安装依赖。
- `worktree:bootstrap` 必须继续遵守 copy-missing-only；目标文件已存在时不得覆盖。
- `worktree:setup` 必须为 root Bun、web Bun、docs Bun、Cargo 各自保存无敏感状态和 input digest；手动 `bun run worktree:setup` 重试 stale 或 failed surface，`bun run worktree:setup -- --force` 强制执行四项。
- 单项失败后必须继续其余任务并记录 failed digest；自动 hook 对相同 failed digest 必须告警并跳过重试，手动入口必须返回非零。
- 未配置 `core.hooksPath` 时，`lefthook` 2.1.7 或更高版本必须在 `PATH` 中可执行；`bun run hooks:install` 缺少或版本过低时必须明确返回非零。已配置 `core.hooksPath` 时安装入口必须安全 no-op，不要求 Lefthook。
- `hooks:install` 不得覆盖 `core.hooksPath` 或 unmanaged 本地 hook；仅当已有 hook 与当前配置生成的 Lefthook 模板及本仓库 marker 逐字相等时才能更新。`prepare-commit-msg` 仅在与带该 hook 配置生成的 Lefthook 标准模板逐字相等、未配置且不是 symlink 时删除。
- 资源同步锁必须位于当前 worktree 的 Git metadata；采用随持锁进程退出自动释放的 advisory lock，同一 worktree busy 时必须非阻塞跳过，不同 linked worktree 不得互相等待。setup 对同一 worktree 自动路径同样非阻塞，手动入口串行等待。
- smoke test 必须使用 fake Bun/Cargo 验证上述调用链，且不得真实联网安装依赖。

### SHOULD

- setup 脚本保持薄封装，优先复用 Bun、Cargo 与现有 lockfiles；状态文件只记录 surface、digest 与结果，不记录本地资源内容。
- 文档应明确主 worktree 与 linked worktree 的触发差异，以及自动/手动失败码语义。

### COULD

- 后续可在 setup 中增加轻量健康检查，但必须保持显式触发。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 新建或失效的 linked worktree 中，Lefthook 生成的 shared `post-checkout` hook 调用当前 checkout 的 bootstrap runner；runner 同步 manifest 中缺失的本地资源，并按 surface 调用依赖 setup。
- root Bun、web Bun、docs Bun、Cargo 分别由其 manifest、lockfile 与工具链输入建立 digest。仅 invalid surface 在 repo root、`web`、`docs-site` 运行 `bun install --frozen-lockfile`，或在 repo root 运行 `cargo fetch --locked`。
- 主 worktree 的 `post-checkout` 只同步本地资源，不运行依赖 setup。
- 自动 hook 忽略依赖 setup 的最终失败码并打印补救提示；手动 `bun run worktree:bootstrap` 保留失败码。

### Edge cases / errors

- 当前 worktree 已存在 `.env.local` 时，bootstrap 必须跳过且不覆盖。
- manifest 直接 Bun package、`Cargo.lock` registry archive、依赖目录不存在、输入 digest 改变或手动强制执行时，由对应 locked install 命令负责恢复。
- 自动失败记录只抑制相同 digest；输入变化后自动路径必须重新尝试。
- 若当前 revision 缺少 bootstrap 脚本，Lefthook command 必须安全 no-op，不能让 checkout 失败。
- staged formatter 必须拒绝任一路径组件中的 symlink，不能向 worktree 外的目标写入。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name）                 | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes）                              |
| ---------------------------- | ------------ | ------------- | -------------- | ------------------------ | --------------- | ------------------- | ------------------------------------------ |
| `bun run worktree:setup`     | CLI          | internal      | Modify         | None                     | repo tooling    | contributors        | 按指纹恢复四项依赖；`--force` 强制全部执行 |
| `bun run worktree:bootstrap` | CLI          | internal      | Modify         | None                     | repo tooling    | contributors        | 同步资源并聚合依赖恢复失败                 |
| `post-checkout` bootstrap    | Git hook     | internal      | Modify         | None                     | repo tooling    | linked worktrees    | Lefthook 触发按指纹恢复，不阻断 checkout   |

### 契约文档（按 Kind 拆分）

- None

## 验收标准（Acceptance Criteria）

- Given 新 linked worktree 缺失 `.env.local` 和依赖目录
  When shared `post-checkout` hook 触发
  Then worktree 获得缺失 `.env.local`，并执行三项 frozen Bun install 与一次 locked Cargo fetch。

- Given linked worktree 的四项依赖均存在且 digest 未变
  When shared `post-checkout` hook 再次触发
  Then 不执行 Bun 或 Cargo；单个缺失或输入变化只恢复对应 surface。

- Given 主 worktree 触发 `post-checkout`
  When setup 脚本运行
  Then 不执行任何 Bun 或 Cargo 依赖命令。

- Given 任一 Bun/Cargo 依赖任务失败
  When 自动 hook 继续执行
  Then 其余任务仍执行、输出失败摘要且 hook 返回 0；相同 digest 的后续自动路径不重试，手动 bootstrap 返回非零并重试。

- Given CI 运行 worktree 与 hook smoke
  When tooling job 安装锁定的全局 Lefthook，真实 repo 外 Lefthook 触发标准 hook、fake `bun` 和 fake `cargo` 捕获 setup 调用链
  Then 测试不联网且能证明主 worktree no-op、选择性恢复、单个 Bun package 与 Cargo archive 缺失恢复、失败抑制、手动重试、force、per-worktree advisory 锁、copy-missing-only、无敏感状态与外置 Lefthook 前置条件。

## 验收清单（Acceptance checklist）

- [x] Lefthook 触发的首次 linked 自动依赖恢复、重复 checkout 跳过与主 worktree 跳过已由 smoke test 覆盖。
- [x] locked Bun/Cargo 选择性安装、Cargo cache 缺失恢复、失败隔离、failed digest 抑制、手动重试与 force 已由 fake `bun`/`cargo` 覆盖。
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

- 风险：把依赖安装放进 linked checkout hook 会增加网络和耗时；本规范以 per-surface digest、manifest 直接 Bun package 与 Cargo registry archive 集合检查限制恢复范围，失败不阻断 checkout。
- 风险：新 linked worktree 在 hook 启动前没有本地 `node_modules`，因此依赖全局 `lefthook`；缺少该命令时安装入口必须尽早失败并给出补救提示。
- 风险：贡献者可能已有自定义 Git hook；安装入口必须跳过 unmanaged hook，避免 Lefthook 将其移动为 `.old` 后停止执行。
- 假设：Bun 是仓库唯一 JS package manager，且 root、`web/`、`docs-site/` 都由 Bun 管理。

## 参考（References）

- `docs/archive/specs/v7se4-worktree-bootstrap/SPEC.md`
- `README.md`
- `AGENTS.md`
- `scripts/test-worktree-bootstrap.sh`
