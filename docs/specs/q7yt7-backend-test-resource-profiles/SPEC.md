# 后端测试资源分层模块化与运行时预算（#q7yt7）

> 当前有效规范以本文为准；实现覆盖与当前状态见 `./IMPLEMENTATION.md`，关键演进原因见 `./HISTORY.md`。

## 背景 / 问题陈述

- 前序工作已将后端测试入口从 `include!()`、编号切片和 `#[path]` 聚合迁入 resource-profile 模块树；后续性能收敛必须以这一稳定分组为真相源。
- 三个 backend required checks 已按资源类型拆分，但 `CI Main` run `31706131099` 的 Stateful SQLite 路径仍耗时 `617s`（编译 `143s`、测试执行 `404s`）。后续优化使 Stateful profile 本身恢复到预算内，但 PR run `31825458818` 的 required jobs 仍有 `Lint & Format Check` `348s`、三个 backend jobs `277s` / `324s` / `382s` 与 `Build Artifacts` `537s`，全量反馈仍然过慢。
- 如果不冻结新的测试组织与 CI 合同，后续对 nextest 分组、fixture 压缩或 top offenders 收口都会继续建立在脆弱的旧切片命名上。

## 目标 / 非目标

### Goals

- 为后端测试建立稳定的 resource-profile 顶层组织：`lightweight`、`stateful_sqlite`、`archive_file_io`。
- 将 `src/tests/slices` 与 `src/upstream_accounts/tests` 都收口到真实模块树，移除 `pool_failover_window_*`、`tests_part_*` 和 `#[path = "../..."]` 聚合。
- 把 owner-facing backend required checks 从单个 `Backend Tests` 改成三个稳定 job，并让质量门禁与发布链路以这三个名称为真相源。
- 将 PR 全部 required job 的 job wall time 恢复到 `<= 180s`，并使 required runner 总秒数相对 run `31825458818` 至少下降 `20%`；以同一 PR head 的首轮冷 SHA 与第二轮热 SHA 验证。

### Non-goals

- 不改变任何生产 HTTP/SSE/API/schema/env/CLI/runtime 语义。
- 不把 backend tests 继续拆成四个以上 required jobs。
- 不在本主题内引入独立 benchmark 服务、长期基准数据库或新的发布流程。
- 不按改动路径跳过 required tests，不将全量测试迁移到 main 或定时任务。
- 不改变前端 Vitest、Storybook 或 E2E 的断言和覆盖；本主题只调整它们与 lint、docs/demo build 的 CI 资源边界。

## 范围（Scope）

### In scope

- `src/tests/**` 与 `src/upstream_accounts/tests**` 的测试模块树重组。
- `.github/scripts/run-backend-tests.sh` 的 profile-aware runner 合同。
- Stateful retry/backoff、no-available-account wait 和大请求 replay fixture 的 test-only 时间/阈值注入点。
- `CI PR` / `CI Main` / `quality-gates` / `release snapshot` / `release gate` 中 required-check 名称、职责拆分与期望 job 集；archive 仅可作为受控实验，不能新增 required check。
- 与本主题直接相关的性能经验文档更新。

### Out of scope

- 生产模块边界、路由行为、数据库 schema 或 UI/Storybook。
- 无关的测试功能修复或新功能扩展。
- 生产 release workflow、发布镜像或前端测试行为本身的性能优化。

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
- PR 中所有 required jobs 必须以 job `startedAt` 至 `completedAt` 计时，首轮冷 SHA 与第二轮热 SHA 均须 `<= 180s`；required runner 总秒数须不高于 run `31825458818` 的 `80%`。
- Cargo registry/git 与 `target` cache 必须分离；nextest target cache key 必须同时绑定 `Cargo.lock`、`Cargo.toml` 与 `src/**/*.rs`，并保留仅按 lockfile 的 restore prefix。迁移时可用原三路径集合只读恢复既有 lockfile-only cache 作为 ancestor seed；clippy 不得写入或争用 nextest target namespace。
- PR Docker smoke 必须只使用 CI 生成的当前 binary 与 web bundle 构建私有 runtime target，并继续运行真实容器 smoke；耗时的 binary、web bundle 与 Xray staging 可以由显式 auxiliary producer 生成，`Build Artifacts` 必须在 producer 失败时实际运行并失败，不得被标记为 skipped。该私有 target 的运行库必须兼容 host-built binary。生产 release workflow、默认 Docker target 和 release profile 不得改变。
- test-only Cargo profile 可关闭 debug info；任何编译参数实验必须保留 source-key target cache 的依赖复用，并且不得进入 production release profile 或运行时配置面。
- `Lint & Format Check`、`Repository Tooling Checks`、`Front-end Tests`、`Storybook Accessibility Tests`、`Docs & Web Demo Build`、`Records Overlay E2E` 和三个 backend profiles 均为 required checks；拆分只能改变资源边界，不得删除测试或断言。无共享状态的 Playwright spec 可以使用有上限的 worker 并行，但必须保留每个 test 的独立 browser context、报告和结果目录。
- retry/backoff、no-available-account wait 和 replay memory threshold 的测试加速只能经私有或 `cfg(test)` seam 注入；生产默认值、尝试次数/顺序、错误分类和运行时配置面不得变化。
- Stateful 的候选线程数必须在完整 profile 的 `4`、`6`、`8` threads 各至少两次热运行中比较；选择最快档位 `10%` 以内的最低线程数。
- 若 archive workflow 被提议保留，它必须作为不进入 branch protection 的显式 auxiliary producer；三个原有 backend required checks 必须完整回放同一 PR head 的 archive。只有同一 PR head 连续两次满足 Stateful `<= 390s`、所有 required job `<= 180s`，且 backend-related jobs 的总 runner 秒数相对 `1257s` 基线下降至少 `20%`（即 `<= 1005s`）时才可保留。

### SHOULD

- bucket 内文件与目录命名应按 failover、routing、archive、stats、maintenance、usage 等语义场景组织，而不是按提交历史或行数残留命名。
- DB-only 测试应优先改为唯一命名的 in-memory SQLite；需要真实 archive/file-path/gzip/write-lock 语义的测试应固定留在 `archive_file_io` bucket。
- Stateful 的非 migration in-memory schema fixture 可由 runner 从一次真实 `ensure_schema` 生成私有 file template，并通过 SQLite backup API 复制到每个唯一 shared-memory SQLite。Archive/File I/O 的普通 current-schema tests 可从同一类私有 template 创建唯一文件副本；采用前必须证明 schema/default-data parity、独立连接可见性、并发写安全和跨测试隔离。legacy migration、文件路径、gzip、文件损坏和 write-lock 测试始终使用真实 `ensure_schema` 和 file fixture。
- 共享 harness、seed、archive helper、SQLite helper 应下沉到稳定测试支撑模块，避免跨大文件复制粘贴。

### COULD

- 在不改变 required-check 数量的前提下，为 profile-aware runner 输出 top offenders 与 profile wall time 摘要，供 PR 证据与后续优化复用。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- profile runner 始终以同一 resource-profile 过滤集合运行；性能优化不得删除用例或改变三个 required check 名称。
- 生产 wrapper 继续使用正式 retry/backoff 与 replay threshold；测试 harness 可为零等待 retry、零等待 no-available-account 和较小的私有 replay threshold 注入值，以验证同一分支而不承担真实时间。
- 需要验证正式时间预算的用例显式清除 retry override；需要验证真实文件语义的用例继续走默认 threshold 与真实文件 fixture。
- 普通 Stateful test state 的 current-schema template 只能由 runner 的真实 fresh schema 生成，SQLite backup 后的 state 仍使用唯一 shared-memory SQLite 与原有多连接池；不得把 shared-memory serialize/deserialize 或逐条 SQL dump 作为最终测试路径。Archive 的普通 file-DB tests 以唯一文件副本获得 current schema，不得把这一路径扩展到 migration 或真实文件语义测试。
- archive build/distribution 是可逆实验。若采用 auxiliary producer，quality-gates 必须显式列出它但不得将它加入 GitHub required checks；仅在同一 PR head 的连续两次 CI 同时满足关键路径与总 runner 成本门槛时，才可保留这条 workflow 拓扑。

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

- Given 同一 PR head 的首轮冷 SHA 与第二轮热 SHA，When 分别计算每个 required job 的 `startedAt` 至 `completedAt`，Then 每个 job 都 `<= 180s`，required runner 总秒数较 run `31825458818` 至少下降 `20%`，且三个 backend profiles 的用例集合不变。

- Given 测试 harness 注入零等待或较小 threshold，When 运行 targeted regression tests，Then 生产默认 delay/threshold、retry attempt/order 和错误分类保持不变，且不存在运行时测试开关。

- Given 运行完整 Stateful profile，When 每个 `4`、`6`、`8` threads 档位运行两次，Then 全部通过，runner 使用最快档位 `10%` 以内的最低线程数。

- Given archive workflow 实验，When 它未同时降低 Stateful 关键路径和 backend runner 总秒数，Then archive workflow 变更不得保留。

## 验收清单（Acceptance checklist）

- [x] 两条测试树都已迁入新的 resource-profile 模块树。
- [x] backend runner 与 CI job 命名合同已冻结并在 docs 中可追溯。
- [x] quality-gates / release snapshot / release gate 已跟随 required-check 变更同步。
- [ ] 当前 PR head 的全部 required jobs 在冷/热两轮均 `<= 180s`，且 runner 总成本满足下降门槛。
- [x] 4/6/8 完整热运行矩阵已记录，且 runner 线程选择符合最低档位规则。
- [x] 测试专用 timing/threshold seam 已验证不改变生产默认行为。
- [ ] archive auxiliary producer 在同一 PR head 的冷/热两轮均满足双重量化门槛；否则从最终 candidate 移除。

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
