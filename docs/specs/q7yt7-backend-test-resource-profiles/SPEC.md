# 后端测试资源分层模块化与运行时预算（#q7yt7）

> 当前有效规范以本文为准；实现覆盖与当前状态见 `./IMPLEMENTATION.md`，关键演进原因见 `./HISTORY.md`。

## 背景 / 问题陈述

- 前序工作已将后端测试入口从 `include!()`、编号切片和 `#[path]` 聚合迁入 resource-profile 模块树；后续性能收敛必须以这一稳定分组为真相源。
- 三个 backend required checks 已按资源类型拆分，但 `CI Main` run `31706131099` 的 Stateful SQLite 路径仍耗时 `617s`（编译 `143s`、测试执行 `404s`），无法满足 PR 的快速反馈需要。
- 如果不冻结新的测试组织与 CI 合同，后续对 nextest 分组、fixture 压缩或 top offenders 收口都会继续建立在脆弱的旧切片命名上。

## 目标 / 非目标

### Goals

- 为后端测试建立稳定的 resource-profile 顶层组织：`lightweight`、`stateful_sqlite`、`archive_file_io`。
- 将 `src/tests/slices` 与 `src/upstream_accounts/tests` 都收口到真实模块树，移除 `pool_failover_window_*`、`tests_part_*` 和 `#[path = "../..."]` 聚合。
- 把 owner-facing backend required checks 从单个 `Backend Tests` 改成三个稳定 job，并让质量门禁与发布链路以这三个名称为真相源。
- 将 Stateful SQLite PR 关键路径恢复到从 workflow 启动至该 required job 完成 `<= 390s`，并以同一 PR head 的连续两次 CI 运行验证。

### Non-goals

- 不改变任何生产 HTTP/SSE/API/schema/env/CLI/runtime 语义。
- 不把 backend tests 继续拆成四个以上 required jobs。
- 不在本主题内引入独立 benchmark 服务、长期基准数据库或新的发布流程。
- 不按改动路径跳过 required tests，不将全量测试迁移到 main 或定时任务。
- 不优化前端 Vitest、Storybook 或 E2E；它们只保留现有基线记录。

## 范围（Scope）

### In scope

- `src/tests/**` 与 `src/upstream_accounts/tests**` 的测试模块树重组。
- `.github/scripts/run-backend-tests.sh` 的 profile-aware runner 合同。
- Stateful retry/backoff、no-available-account wait 和大请求 replay fixture 的 test-only 时间/阈值注入点。
- `CI PR` / `CI Main` / `quality-gates` / `release snapshot` / `release gate` 中 backend test required-check 名称与期望 job 集；archive 仅可作为受控实验，不能新增 required check。
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
- `run-backend-tests.sh` 可接受可选 `--archive-file <path>`，从已有 nextest archive 运行同一 profile 过滤；未提供该参数时必须继续用锁定依赖编译并运行。
- Stateful 运行时优化必须以从 workflow 启动到 `Backend Tests (Stateful SQLite)` 完成的 wall time 为主指标；同一 PR head 连续两次均须 `<= 390s`。
- retry/backoff、no-available-account wait 和 replay memory threshold 的测试加速只能经私有或 `cfg(test)` seam 注入；生产默认值、尝试次数/顺序、错误分类和运行时配置面不得变化。
- Stateful 的候选线程数必须在完整 profile 的 `4`、`6`、`8` threads 各至少两次热运行中比较；选择最快档位 `10%` 以内的最低线程数。
- 若 archive workflow 被提议保留，必须在同一 PR head 连续两次满足 Stateful `<= 390s`，且 backend-related jobs 的总 runner 秒数相对 `1257s` 基线下降至少 `20%`（即 `<= 1005s`）；否则不得进入最终 candidate。

### SHOULD

- bucket 内文件与目录命名应按 failover、routing、archive、stats、maintenance、usage 等语义场景组织，而不是按提交历史或行数残留命名。
- DB-only 测试应优先改为唯一命名的 in-memory SQLite；需要真实 archive/file-path/gzip/write-lock 语义的测试应固定留在 `archive_file_io` bucket。
- 非 migration 的 in-memory schema fixture 可由 runner 从一次真实 `ensure_schema` 生成私有 file template，并通过 SQLite backup API 复制到每个唯一 shared-memory SQLite；采用前必须证明 schema/default-data parity、独立连接可见性、并发写安全和跨测试隔离。legacy migration、文件路径、gzip 和 write-lock 测试始终使用真实 `ensure_schema` 和 file fixture。
- 共享 harness、seed、archive helper、SQLite helper 应下沉到稳定测试支撑模块，避免跨大文件复制粘贴。

### COULD

- 在不改变 required-check 数量的前提下，为 profile-aware runner 输出 top offenders 与 profile wall time 摘要，供 PR 证据与后续优化复用。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- profile runner 始终以同一 resource-profile 过滤集合运行；性能优化不得删除用例或改变三个 required check 名称。
- 生产 wrapper 继续使用正式 retry/backoff 与 replay threshold；测试 harness 可为零等待 retry、零等待 no-available-account 和较小的私有 replay threshold 注入值，以验证同一分支而不承担真实时间。
- 需要验证正式时间预算的用例显式清除 retry override；需要验证真实文件语义的用例继续走默认 threshold 与真实文件 fixture。
- 普通 Stateful test state 的 current-schema template 只能由 runner 的真实 fresh schema 生成，SQLite backup 后的 state 仍使用唯一 shared-memory SQLite 与原有多连接池；不得把 shared-memory serialize/deserialize、逐条 SQL dump 或直接文件副本原型作为最终测试路径。
- archive build/distribution 是可逆实验。仅在同一 PR head 的连续两次 CI 同时满足关键路径与总 runner 成本门槛时，才同步 workflow 与 quality-gates 合同；否则 runner 的可选 archive 入口不改变 required CI 拓扑。

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

- Given 同一 PR head 的两次完整 CI，When 分别计算 workflow start 至 Stateful SQLite job completed 的时长，Then 两次都 `<= 390s`，且三个 backend profiles 的用例集合不变。

- Given 测试 harness 注入零等待或较小 threshold，When 运行 targeted regression tests，Then 生产默认 delay/threshold、retry attempt/order 和错误分类保持不变，且不存在运行时测试开关。

- Given 运行完整 Stateful profile，When 每个 `4`、`6`、`8` threads 档位运行两次，Then 全部通过，runner 使用最快档位 `10%` 以内的最低线程数。

- Given archive workflow 实验，When 它未同时降低 Stateful 关键路径和 backend runner 总秒数，Then archive workflow 变更不得保留。

## 验收清单（Acceptance checklist）

- [x] 两条测试树都已迁入新的 resource-profile 模块树。
- [x] backend runner 与 CI job 命名合同已冻结并在 docs 中可追溯。
- [x] quality-gates / release snapshot / release gate 已跟随 required-check 变更同步。
- [ ] Stateful PR 关键路径连续两次 `<= 390s`。
- [x] 4/6/8 完整热运行矩阵已记录，且 runner 线程选择符合最低档位规则。
- [x] 测试专用 timing/threshold seam 已验证不改变生产默认行为。
- [x] archive 仅在双重量化门槛通过时进入 CI candidate；当前 candidate 不包含 archive workflow。

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- Unit tests: 三个 backend profiles 必须各自通过。
- Integration tests: `cargo test` 与相关 shared-testbox smoke 必须保持通过。
- E2E tests (if applicable): None

### UI / Storybook (if applicable)

- None

### Quality checks

- `cargo fmt --all -- --check`
- `cargo clippy --locked --all-targets --all-features -- -D warnings`
- `cargo check --locked --all-targets --all-features`
- `bash .github/scripts/test-quality-gates-contract.sh`
- `bash .github/scripts/test-live-quality-gates.sh`

## Visual Evidence

## Related PRs

- None

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：按 resource-profile 拆 required jobs 会同步改动 quality-gates 与 release 合同，若 job 名称漂移会直接阻断 PR merge 与 release。
- 风险：schema 模板在 shared in-memory 连接池和文件副本下可能引入连接可见性或 SQLite snapshot lock 回归；没有完整等价性证据时必须回退到真实 `ensure_schema`。
- 风险：archive 可能减少重复编译却增加 Stateful job 的 dependency critical path；必须以 workflow-start wall time 而非单个 runner 命令判断。
- 假设（已确定）：每个 PR 保持全部 required tests，性能优化只改变测试内部成本与受控 runner 复用。

## 参考（References）

- `../4tgau-backend-structure-followup/SPEC.md`
- `../4tgau-backend-structure-followup/IMPLEMENTATION.md`
- `../../solutions/performance/rust-backend-test-runtime-feedback-loop.md`
