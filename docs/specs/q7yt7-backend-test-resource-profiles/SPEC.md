# 后端测试资源分层模块化与运行时预算（#q7yt7）

> 当前有效规范以本文为准；实现覆盖与当前状态见 `./IMPLEMENTATION.md`，关键演进原因见 `./HISTORY.md`。

## 背景 / 问题陈述

- 后端测试入口虽然已经摆脱 `include!()`，但 `src/tests/slices` 与 `src/upstream_accounts/tests_part_*` 仍然由超大文件、编号切片、`#[path]` 聚合和大范围 `pub(crate) use` 组成。
- 当前 `Backend Tests` 仍以单个 required check 暴露给 owner；一旦运行时回归，只能看到总耗时变慢，缺乏稳定的资源/成本分层诊断。
- 如果不冻结新的测试组织与 CI 合同，后续对 nextest 分组、fixture 压缩或 top offenders 收口都会继续建立在脆弱的旧切片命名上。

## 目标 / 非目标

### Goals

- 为后端测试建立稳定的 resource-profile 顶层组织：`lightweight`、`stateful_sqlite`、`archive_file_io`。
- 将 `src/tests/slices` 与 `src/upstream_accounts/tests` 都收口到真实模块树，移除 `pool_failover_window_*`、`tests_part_*` 和 `#[path = "../..."]` 聚合。
- 把 owner-facing backend required checks 从单个 `Backend Tests` 改成三个稳定 job，并让质量门禁与发布链路以这三个名称为真相源。
- 将运行时优化目标固定为 `CI Main` 中最慢 backend required job 的 wall time `<= 6m30s`。

### Non-goals

- 不改变任何生产 HTTP/SSE/API/schema/env/CLI/runtime 语义。
- 不把 backend tests 继续拆成四个以上 required jobs。
- 不在本主题内引入独立 benchmark 服务、长期基准数据库或新的发布流程。

## 范围（Scope）

### In scope

- `src/tests/**` 与 `src/upstream_accounts/tests**` 的测试模块树重组。
- `.github/scripts/run-backend-tests.sh` 的 profile-aware runner 合同。
- `CI PR` / `CI Main` / `quality-gates` / `release snapshot` / `release gate` 中 backend test required-check 名称与期望 job 集。
- 与本主题直接相关的性能经验文档更新。

### Out of scope

- 生产模块边界、路由行为、数据库 schema 或 UI/Storybook。
- 无关的测试功能修复或新功能扩展。
- 后端测试以外的 CI job 拆分。

## 需求（Requirements）

### MUST

- 顶层测试 bucket 固定为 `lightweight`、`stateful_sqlite`、`archive_file_io`，不得继续使用编号或字母切片作为长期命名。
- `src/upstream_accounts/tests` 不得再通过 `#[path = "../tests_part_X.rs"]` 引入外部编号分片。
- backend required checks 必须精确为：
  - `Backend Tests (Lightweight)`
  - `Backend Tests (Stateful SQLite)`
  - `Backend Tests (Archive / File I/O)`
- `run-backend-tests.sh` 必须提供稳定 `--profile` 入口，供本地与 CI 复用同一分组真相。
- 运行时优化必须以 `CI Main` 中最慢 backend required job 的 wall time 为主指标，目标为 `<= 6m30s`。

### SHOULD

- bucket 内文件与目录命名应按 failover、routing、archive、stats、maintenance、usage 等语义场景组织，而不是按提交历史或行数残留命名。
- DB-only 测试应优先改为唯一命名的 in-memory SQLite；需要真实 archive/file-path/gzip/write-lock 语义的测试应固定留在 `archive_file_io` bucket。
- 共享 harness、seed、archive helper、SQLite helper 应下沉到稳定测试支撑模块，避免跨大文件复制粘贴。

### COULD

- 在不改变 required-check 数量的前提下，为 profile-aware runner 输出 top offenders 与 profile wall time 摘要，供 PR 证据与后续优化复用。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- PR1：先完成测试树模块化。实现后，新的模块路径与语义命名成为后续 nextest/profile 过滤的唯一真相源。
- PR2：在 PR1 的模块路径稳定后，引入 profile-aware runner，并把 CI/quality-gates/release 相关契约切到三个 backend required jobs。
- 运行时优化仅能通过测试组织、fixture、SQLite 连接池、archive seed 范围、nextest 分组和 CI job 拆分达成；不得通过放宽生产语义验证来“做快”。

### Edge cases / errors

- 若现有测试名称冲突导致按模块/名称过滤不稳定，可做最小必要的测试名重命名，但必须保持被测行为与断言语义等价。
- 若某类 archive/file-path 测试无法安全切到 in-memory SQLite，必须显式保留在 `archive_file_io` bucket，而不是为了命中预算偷偷降覆盖。
- 若 CI required-check 名称变化，所有 quality-gates、release snapshot 和相关自测 fixtures 必须同轮更新，禁止留下“CI 能跑、门禁却认旧名字”的半迁移状态。

## 接口契约（Interfaces & Contracts）

### 接口清单（Inventory）

| 接口（Name）                                               | 类型（Kind） | 范围（Scope） | 变更（Change） | 契约文档（Contract Doc） | 负责人（Owner） | 使用方（Consumers）                        | 备注（Notes）              |
| ---------------------------------------------------------- | ------------ | ------------- | -------------- | ------------------------ | --------------- | ------------------------------------------ | -------------------------- |
| `.github/scripts/run-backend-tests.sh --profile <profile>` | cli          | internal      | Modify         | None                     | backend/ci      | local dev, CI PR, CI Main                  | 新增稳定 profile 入口      |
| `Backend Tests (Lightweight)`                              | workflow-job | external      | New            | None                     | ci              | GitHub branch protection, release snapshot | 替换旧单一 backend check   |
| `Backend Tests (Stateful SQLite)`                          | workflow-job | external      | New            | None                     | ci              | GitHub branch protection, release snapshot | 替换旧单一 backend check   |
| `Backend Tests (Archive / File I/O)`                       | workflow-job | external      | New            | None                     | ci              | GitHub branch protection, release snapshot | 替换旧单一 backend check   |
| `Backend Tests`                                            | workflow-job | external      | Delete         | None                     | ci              | GitHub branch protection, release snapshot | 旧单一 required check 退场 |

### 契约文档（按 Kind 拆分）

- `None`

## 验收标准（Acceptance Criteria）

- Given 当前后端测试树仍包含 `pool_failover_window_[a-k]` 与 `tests_part_[1-7]`，When 完成 PR1，Then 这些旧切片文件名与 `#[path]` 聚合路径都不再存在，测试入口改为真实模块树。

- Given 本地或 CI 需要运行 backend tests，When 调用 `bash .github/scripts/run-backend-tests.sh --profile lightweight|stateful-sqlite|archive-file-io`，Then 三个 profile 都能独立通过并复用同一分组真相。

- Given `CI PR` 与 `CI Main` 已更新，When GitHub 评估 required checks，Then backend required checks 只包含三个新 job 名称，不再引用旧 `Backend Tests`。

- Given 运行时优化收口完成，When 查看 `CI Main` 中三个 backend required jobs，Then 最慢 job 的 wall time `<= 6m30s`。

## 验收清单（Acceptance checklist）

- [ ] 两条测试树都已迁入新的 resource-profile 模块树。
- [ ] backend runner 与 CI job 命名合同已冻结并在 docs 中可追溯。
- [ ] quality-gates / release snapshot / release gate 已跟随 required-check 变更同步。
- [ ] 运行时预算口径与通过阈值已明确。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: 三个 backend profiles 必须各自通过。
- Integration tests: `cargo test` 与相关 shared-testbox smoke 必须保持通过。
- E2E tests (if applicable): None

### UI / Storybook (if applicable)

- None

### Quality checks

- `cargo fmt --all -- --check`
- `cargo check --locked --all-targets --all-features`
- `bash .github/scripts/test-quality-gates-contract.sh`
- `bash .github/scripts/test-release-snapshot.sh`
- `bash .github/scripts/test-live-quality-gates.sh`

## Visual Evidence

## Related PRs

- None

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：按 resource-profile 拆 required jobs 会同步改动 quality-gates 与 release 合同，若 job 名称漂移会直接阻断 PR merge 与 release。
- 风险：部分 stateful/archive 测试可能共享隐式 helper 或 fixture 全局状态，模块化后会暴露出此前被大文件顺序掩盖的耦合。
- 假设（已确定）：第二个 PR 可以建立在第一个 PR 的新模块路径真相源之上，不再额外保留旧切片兼容层。

## 参考（References）

- `../4tgau-backend-structure-followup/SPEC.md`
- `../4tgau-backend-structure-followup/IMPLEMENTATION.md`
- `../../solutions/performance/rust-backend-test-runtime-feedback-loop.md`
