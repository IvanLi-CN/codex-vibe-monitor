# Worktree bootstrap 同步开发环境配置（#v7se4）

## 状态

- Status: 待实现
- Created: 2026-03-14
- Last: 2026-03-14

## 背景 / 问题陈述

- 当前仓库已经固定了 Bun-first、`lefthook.yml`、`.env.local` 与 linked worktree 的使用方式，但新 worktree 仍需要手工补本地配置，容易漏掉环境文件。
- 现有仓库没有 `worktree bootstrap` 入口，也没有把自动同步做成 shared Git hook contract，导致 linked worktree 之间的开发机配置不能稳定复用。
- `$style-playbook` 的 `worktree-bootstrap` 参考把这类能力定义为“共享 hooks + checkout 自动触发 + copy-missing-only + CI smoke”的仓库契约；当前仓库缺少这层约束。

## 目标 / 非目标

### Goals

- 为当前仓库补齐 shared Git hooks 驱动的 worktree bootstrap，让 linked worktree 自动拿到缺失的本地开发配置。
- 提供自动与手动双入口：`post-checkout` 自动同步，`bun run worktree:bootstrap` 可手动重跑。
- 首版同步范围只包含被忽略的 `.env.local`，并严格保持 copy-missing-only，不覆盖目标 worktree 已存在的本地文件。
- 用真实 linked worktree smoke test 把这条路径纳入 CI，避免后续回归。

### Non-goals

- 不同步 `node_modules`、`web/node_modules`、SQLite DB、`.codex/xray-forward` 等运行态或大体积目录。
- 不修改任何 HTTP API、SSE、数据库 schema 或前端业务逻辑。
- 不引入需要联网安装或 checkout 时执行重型命令的自动动作。

## 范围（Scope）

### In scope

- `docs/specs/v7se4-worktree-bootstrap/SPEC.md`
- `docs/specs/README.md`
- `package.json`
- `bun.lock`
- `lefthook.yml`
- `scripts/install-hooks.sh`
- `scripts/run-lefthook-hook.sh`
- `scripts/worktree-bootstrap.sh`
- `scripts/sync-worktree-resources.sh`
- `scripts/worktree-sync.paths`
- `scripts/test-worktree-bootstrap.sh`
- `README.md`
- `AGENTS.md`
- `.github/workflows/ci.yml`

### Out of scope

- `src/**`
- `web/src/**`
- `.env.local` 内容本身
- 本地依赖安装自动化（例如 checkout 时自动跑 `bun install`）

## 需求（Requirements）

### MUST

- `bun run hooks:install` 必须把 hook 链安装到 shared Git hooks 层，而不是某个单独 worktree 私有目录。
- `post-checkout` 必须在 linked worktree 可用，并在目标 worktree 缺失 `.env.local` 时自动复制主 worktree 的同名文件。
- 自动与手动同步都必须遵守 copy-missing-only；目标文件已存在时不得覆盖。
- 若存在外部自定义 `core.hooksPath`，安装脚本必须 warning + exit 0，不改写他人的 hook 目录。
- 若 shared hooks 目录中已存在未知来源的 hook 文件，安装脚本必须保留原文件并仅对该 hook 打 warning，不得静默覆盖。
- 若当前 revision 缺少 bootstrap 脚本/manifest，hook 必须安全 no-op，不能让 checkout 失败。
- CI 必须执行真实 linked worktree smoke test。

### SHOULD

- root tooling 使用 repo-local `lefthook` 依赖，避免依赖开发机全局安装。
- 同步清单用受版本控制的 manifest 文件维护，后续扩表时不需要改同步语义。

### COULD

- 在同步脚本中预留内部 override 环境变量，便于测试夹具注入 source/target 路径。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- 在任一 worktree 执行 `bun install && bun run hooks:install` 后，shared Git hooks 目录会安装 `pre-commit`、`commit-msg`、`post-checkout` 三个 wrapper，wrapper 统一回到当前 checkout 的仓库根目录寻找 `scripts/run-lefthook-hook.sh`。
- 当新 linked worktree 被 checkout/add 时，shared `post-checkout` wrapper 会直接回到当前 checkout 的仓库根目录执行 bootstrap runner；若当前 revision 里的 `scripts/sync-worktree-resources.sh` 不存在或不可执行，则静默退出。
- `scripts/sync-worktree-resources.sh` 默认把 `dirname("$(git rev-parse --git-common-dir)")` 视为主 worktree 根目录，把 `git rev-parse --show-toplevel` 视为当前 worktree 根目录，并按 `scripts/worktree-sync.paths` 逐项复制缺失资源。
- `bun run worktree:bootstrap` 会先执行 `hooks:install`，再执行同步脚本，作为幂等的手动补跑入口。

### Edge cases / errors

- 当当前 worktree 就是主 worktree 时，同步脚本直接 no-op。
- 当主 worktree 缺少 `.env.local` 时，同步脚本只打印提示，不失败。
- 当脚本从仓库外通过绝对路径调用时，`git-common-dir` 的相对返回值仍必须锚定到目标 repo 根目录，不能错误读取调用者当前目录。
- 当目标 worktree 已有 `.env.local` 时，自动/手动 bootstrap 都必须跳过，不覆盖。
- 当切换到不包含 runner/sync 脚本的历史 commit 时，shared hook wrapper 必须因为找不到 repo 脚本而直接 exit 0。
- 当 repo 设置了自定义 `core.hooksPath` 时，`hooks:install` 必须保留现状，不写入 managed marker。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name）                  | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers） | 备注（Notes）         |
| ----------------------------- | ------------ | ------------- | -------------- | ------------------------ | --------------- | ------------------- | --------------------- |
| `bun run hooks:install`       | CLI          | internal      | New            | None                     | repo tooling    | contributors        | 安装 shared Git hooks |
| `bun run worktree:bootstrap`  | CLI          | internal      | New            | None                     | repo tooling    | contributors        | 手动重跑 bootstrap    |
| `scripts/worktree-sync.paths` | file format  | internal      | New            | None                     | repo tooling    | bootstrap scripts   | 受版本控制的同步清单  |
| `post-checkout` bootstrap     | Git hook     | internal      | New            | None                     | repo tooling    | linked worktrees    | 只补缺失本地资源      |

### 契约文档（按 Kind 拆分）

None

## 验收标准（Acceptance Criteria）

- Given 主 worktree 存在 `.env.local`
  When 创建并 checkout 一个新的 linked worktree
  Then 新 worktree 会自动获得缺失的 `.env.local`。
- Given 目标 worktree 已存在 `.env.local`
  When 自动 bootstrap 或手动执行 `bun run worktree:bootstrap`
  Then 目标文件内容保持不变。
- Given 执行 `bun run hooks:install`
  When 检查 `git rev-parse --git-path hooks`
  Then `pre-commit`、`commit-msg`、`post-checkout` wrapper 带有 managed marker，且位于 shared hooks 目录。
- Given repo 配置了外部 `core.hooksPath`
  When 执行 `bun run hooks:install`
  Then 命令退出码为 0，打印 warning，且不改写该自定义 hooks 目录。
- Given shared hooks 目录中已有未知来源的 hook 文件
  When 执行 `bun run hooks:install`
  Then 该 hook 文件保持原样，安装脚本仅对该 hook 输出 warning，并继续处理其他可托管 hook。
- Given 已安装 shared hooks
  When checkout 到不包含 bootstrap 脚本的历史 revision
  Then checkout 成功且 hook 安全 no-op。
- Given CI 运行 `scripts/test-worktree-bootstrap.sh`
  When linked worktree smoke 流程执行
  Then 上述关键路径全部通过。

## 实现前置条件（Definition of Ready / Preconditions）

- [x] 目标/非目标与同步边界已冻结。
- [x] 自动入口、手动入口与 copy-missing-only 语义已确定。
- [x] 接口契约明确为内部 CLI / Git hook，不涉及业务接口变更。
- [x] 快车道交付授权已确认。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `bun install`
- `bash scripts/test-worktree-bootstrap.sh`
- `bun run check:bun-first`

### Quality checks

- 继续保持现有 `Lint & Format Check` job 名称不变。
- 不新增业务代码验证范围之外的 required check 名称。

## 文档更新（Docs to Update）

- `README.md`: 增加 worktree bootstrap 的首次安装、自动行为与手动补跑说明。
- `AGENTS.md`: 增加 repo-level hook/bootstrap 命令与 linked worktree 行为说明。
- `docs/specs/README.md`: 登记该 spec。

## 计划资产（Plan assets）

None

## Visual Evidence (PR)

None

## 资产晋升（Asset promotion）

None

## 实现里程碑（Milestones / Delivery checklist）

- [ ] M1: 建立 spec/index 并冻结 worktree bootstrap 契约。
- [ ] M2: 完成 shared hook 安装链、bootstrap runner 与 sync manifest。
- [ ] M3: 完成 linked worktree smoke test、CI 接入与文档更新。
- [ ] M4: 完成本地验证、PR 与 review-loop 收敛。

## 方案概述（Approach, high-level）

- 参考 `$style-playbook` 的 `worktree-bootstrap` 默认，把 linked worktree 的本地环境同步定义为 repo contract，而不是个人脚本。
- hook 层使用 shared Git hooks wrapper，避免每个 worktree 各装一遍；实际行为由当前 checkout 的 repo 脚本决定，从而兼容历史 revision。
- 自动路径保持轻量，只复制 manifest 中声明的缺失资源，不执行依赖安装、格式化或其他重型命令。
- smoke test 使用临时独立 repo + real `git worktree add` 验证，避免 CI 与本仓库共享 hooks 状态互相污染。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：`post-checkout` 的执行时机在不同 Git 操作上可能略有差异，因此必须以真实 linked worktree smoke test 锁住行为。
- 风险：部分开发机可能已有自定义 `core.hooksPath`；本计划选择保守退出，不尝试接管。
- 开放问题：None。
- 假设：主 worktree 可稳定由 `git-common-dir` 的父目录推导得到；当前仓库布局满足该假设。

## 变更记录（Change log）

- 2026-03-14: 创建 spec，冻结 shared hooks + `post-checkout` + `.env.local` copy-missing-only + fast-track 的实现口径。

## 参考（References）

- `$style-playbook` tag `worktree-bootstrap`
- `$style-playbook` project snapshot `codex-vibe-monitor`
