---
title: GitHub Actions CI performance optimization
module: ci-workflows
problem_type: performance
component: GitHub Actions CI
tags:
  - github-actions
  - ci
  - workflow
  - performance
  - test-strategy
  - playwright
  - vitest
  - sqlite
  - artifacts
status: active
related_specs:
  - docs/specs/q7yt7-backend-test-resource-profiles/SPEC.md
---

# GitHub Actions CI 性能优化

## 1. 背景与目标

GitHub Actions CI 的慢，不一定来自某一条测试命令。一个 PR run 的时间还包括排队、运行环境准备、依赖和编译产物恢复、测试 fixture、浏览器启动、产物传递、汇总门禁以及失败诊断上传。只看某个 job 或某个本地测试的耗时，不能证明整个 CI 变快。

本 Solution 适用于同时包含前端、后端、浏览器回归、构建产物和质量门禁的仓库。它解决的是流程和执行层级的设计问题：

- 降低 required CI 的墙钟时间，优先缩短 DAG 的关键路径。
- 降低同一变更所需的 runner 总执行时间，减少重复准备、重复编译和重复测试。
- 保持测试意图、覆盖边界、质量门禁和生产行为不变。
- 让性能结论可以复现、对比和回退，而不是依赖一次局部的“跑得更快”。

GitHub Actions 消耗不增加是硬约束，而不是本 Solution 的独立优化目标。并行化、缓存、产物和触发策略都必须同时检查新增 job、重复 run、上传字节数、保留时间和 runner 资源，不能用更高的 Actions 消耗换取表面上的墙钟缩短。

## 2. CI 性能指标与成本不增加约束

### 2.1 指标定义

| 指标                    | 含义                                                            | 使用方式                            |
| ----------------------- | --------------------------------------------------------------- | ----------------------------------- |
| required wall-clock     | 从第一条适用 required job 开始，到最终 required gate 完成的时间 | 判断开发者等待是否缩短              |
| critical path           | workflow DAG 中从触发到 required gate 的最长依赖链              | 判断并行是否真正缩短交付路径        |
| runner total            | 所有适用 job 的 runner 执行秒数之和                             | 判断是否只是把工作复制到更多 runner |
| job wall time           | 单个 job 从启动到结束的时间                                     | 定位 top offender 和资源 profile    |
| queue/setup time        | 排队、runner 分配、依赖安装和环境准备时间                       | 与测试执行时间分开归因              |
| cache/artifact transfer | cache restore、产物上传、下载和解压的耗时与字节数               | 判断复用是否抵消了并行收益          |
| correctness outcome     | 通过、失败、取消、跳过和重试的语义                              | 防止速度提升来自隐藏失败            |

一次性能改动至少应同时报告 required wall-clock、critical path、每个 required job wall time 和 runner total。局部 Vitest、Playwright 或 Stateful SQLite 的变快只能作为定位证据，不能单独作为 CI 优化结论。

### 2.2 Actions 消耗不增加的检查

将候选方案与同一 workload 的基线比较时，必须检查以下边界：

- 同一变更是否触发了更多 workflow run 或更多 job。
- 并行后是否因为 runner 数量增加而使 runner total 上升。
- cache restore/save 是否重复解压同一目录，或把更大的缓存带入关键路径。
- artifact、日志和测试报告是否包含不必要的 `node_modules`、`target`、浏览器缓存或完整工作区。
- retention 是否长于诊断和交接所需的最短期限。
- 失败诊断是否只在失败或明确请求时上传，而不是每次成功运行都保存大文件。

Artifact 是跨 job 的交接载体，不是长期缓存。应只上传 required gate 真正消费的最小文件和失败诊断，使用 run/attempt 作用域避免覆盖，并设置满足排障所需的最短 `retention-days`。缓存应服务于可复用的依赖或不可变构建输入，不应通过上传大目录来替代正确的 producer/consumer 设计。

成本和账单判断以仓库配置、run/job 数量、runner 秒数、缓存/产物字节和 retention 为证据；不要在文档中预设某个价格、折扣或未来账单金额。

## 3. CI 瓶颈识别方法

### 3.1 先拆阶段，再做归因

把一次 CI run 拆成以下阶段，并在 job 和 step 级别记录耗时：

```text
trigger / queue
  -> runner bootstrap and toolchain
  -> dependency and build-cache restore
  -> compile / link / bundle
  -> fixture and test execution
  -> browser server and browser execution
  -> artifact upload/download and aggregation
  -> required gate
```

每个阶段要回答三个问题：它是否在关键路径上，是否被多个 job 重复执行，是否需要真实环境才能验证目标行为。这样可以区分冷编译、热执行、fixture 初始化、真实等待、浏览器启动、产物传输和门禁等待，而不会把所有时间都归类为“测试太多”。

### 3.2 常见瓶颈类别

- **重复工作**：多个 backend job 各自编译相同测试二进制，或多个浏览器 job 各自安装同一版本浏览器。
- **错误串行**：互不依赖的 lint、frontend、docs/demo 和 backend profile 被一条 `needs` 链串起来。
- **资源争用**：多个 Playwright worker 同时压同一个 Vite server，或 Stateful SQLite 的线程数、连接池和 hosted runner CPU 相互放大。
- **错误缓存**：只按 lockfile 命中整个 `target`，恢复了陈旧产物；或者多个 job 先后解包同一 `target`，抵消缓存收益。
- **fixture 过重**：只需要 current-schema 状态的测试每次运行完整 migration、retention 或 archive pipeline。
- **触发过宽**：文档或不相关路径的变更触发昂贵性能、浏览器或完整构建 workflow。
- **失败被隐藏**：producer 失败后 consumer 被自然 skipped，required gate 却仍显示为可通过。
- **诊断过度**：成功 run 上传完整报告、缓存和工作区，增加传输时间与 Actions 消耗。

### 3.3 当前仓库的定位经验

现有 backend Solution 记录过一次可复用的拆分：`CI Main` run `31706131099` 中，Stateful SQLite wall time 为 `617s`，其中 compile 为 `143s`、test execution 为 `404s`；backend-related jobs 合计 `1257s`。这说明不能只优化测试函数，也要分别处理编译、fixture、线程并发和 workflow 关键路径。

同一 Solution 还记录了 PR run `31825458818` 中 Lint `348s`、Lightweight/Stateful/Archive `277s`/`324s`/`382s` 和 Build Artifacts `537s`。这些数字是历史诊断样本，不是当前仓库的永久 SLA；它们的价值在于提醒我们同时看 required job、非关键路径 job 和总 runner 秒数。

每次新实验都应记录：commit SHA、事件类型、runner 类型、工具链、缓存冷热状态、完整 workload、每个 job/step 时间和最终 gate 结果。不同 SHA、不同 runner 或不完整 workload 的数字不能直接比较。

## 4. Workflow 内部设计

### 4.1 用 DAG 表达真正的依赖

推荐的 workflow 形状是：

```text
event
  -> change classification
  -> immutable producer(s)
  -> independent consumers in parallel
  -> fail-closed aggregate gate
  -> required check
```

依赖关系只表达真实数据依赖。构建、前端测试、Storybook、docs/demo 和不同 backend profile 没有共享输入时，应并行启动；需要同一编译产物或同一 fixture 的 job 才通过 producer/consumer 连接。

### 4.2 Producer、Consumer 与 Gate

- **Producer** 在确定的 commit 上一次生成不可变交接物，例如 nextest archive、PR smoke 所需的 debug binary/web bundle 或已验证的测试 fixture。
- **Consumer** 下载并解包一次，然后只执行自己的 profile/filter；不能重新编译或重新生成 producer 已经交付的内容。
- **Gate** 使用 `always()` 检查 producer 和所有 consumer 的真实结果。上游失败必须转化为显式 failed gate，不能依赖 `needs` 造成 skipped 后被误判为成功。

Producer 失败时不能保存部分 `target`、半成品 archive 或不完整的交接 artifact。只有 producer 完整成功后才能发布可复用产物；这条规则同时保护正确性和下一次 run 的冷/热实验可比性。

### 4.3 并行、分片与资源边界

- 按资源 profile 拆分 backend lightweight、Stateful SQLite 和 Archive/File I/O；Stateful SQLite 可以使用 `hash:1/2`、`hash:2/2`，再由一个 fail-closed aggregate gate 汇总。
- 选择线程数时必须对完整 profile 的多个重复运行取证，选取最快档位附近更低的资源档位，而不是只看一次最短结果。
- 不要无上限叠加同一 hosted runner 上的 Playwright worker pool。共享 Vite server 的 suite 应优先固定 worker，并通过 producer/gate 拆分长流程，而不是盲目增加浏览器并发。
- 只并行真正独立的工作；如果多个 job 争用同一 cache、数据库、server 或 runner CPU，增加并行度可能同时拉长墙钟和 runner total。

### 4.4 Cache 与构建产物复用

- 分离 Cargo registry/git cache、Bun 依赖 cache、浏览器版本 cache 和 source-key 的编译产物。
- `target` 或 nextest archive 的 key 必须包含 lockfile、manifest、工具链、profile 和源码 fingerprint；lockfile-only prefix 只能作为受控 fallback，不能作为完整 target 的唯一身份。
- Clippy、Lint 等 job 不应恢复或写入会与 nextest 争用的 legacy `target`。
- 先恢复一个正确的 source-key cache，再决定是否需要 producer；不要在关键路径连续解包两份同类 target。
- cache save 只允许由完整成功的 producer 执行，失败或取消的 run 不应发布部分目录。

缓存命中不是性能结论。必须把 restore、解压、compile/link、test execution 和 job wall time 一起比较，确认复用没有把成本转移到关键路径之外。

### 4.5 并发取消与失败传播

普通 PR candidate 可以按 PR 编号分组并取消旧 candidate，避免同一 PR 的过期 run 占用 runner；但 merge queue、main、release completion 和不可中断的诊断 run 应使用不取消或独立的并发语义。不能把 required gate 和可取消的 candidate 混在同一个 concurrency contract 中。

任何 `paths`、`if`、`needs` 或 `continue-on-error` 改动都必须检查 required check 是否仍然可见、失败是否仍然 fail-closed，以及 merge queue 是否覆盖同一质量边界。

## 5. 按需触发策略

### 5.1 按 run class 设计触发

| run class              | 典型触发                            | 默认目标                        | 并发语义                      |
| ---------------------- | ----------------------------------- | ------------------------------- | ----------------------------- |
| PR candidate           | `pull_request`                      | 快速反馈和 required checks      | 同一 PR 的旧 candidate 可取消 |
| merge queue            | `merge_group`                       | 对最终合并候选重新验证          | 不取消 required completion    |
| Main                   | `push` 到 `main`                    | 主干完整验证和后续发布输入      | 不取消                        |
| Performance/diagnostic | `workflow_dispatch` 或明确 schedule | 受控的完整 workload、A/B 或诊断 | 不取消，绑定明确 SHA          |

当前仓库的 `CI PR` 同时处理 PR 和 merge queue，`CI Main` 处理主干，`Observability Performance` 是手动性能入口。这个分工应继续保持：昂贵的性能诊断不应隐式变成每个 PR 的 required job，但一旦进入 required 或 merge queue 路径，就必须有明确的 gate 和失败语义。

### 5.2 路径与变更分类

按需触发可以按变更域选择 workload，例如文档、前端、后端、workflow/quality-gates 和性能诊断分别拥有不同的验证集合。分类规则必须满足：

- workflow、测试合同、质量门禁、构建输入和共享 fixture 改动触发完整相关验证。
- 不相关的文档或 demo 变更不自动启动昂贵的性能诊断。
- 被 branch protection 要求的 check 不能因为 path filter 直接消失并长期 Pending；需要保留可见的 no-op/aggregate gate，并明确它为什么不适用。
- PR candidate、merge queue 和 Main 的结果不能靠“这次没跑”假设等价；适用性必须在 gate 中可观察。

### 5.3 防止重复触发与触发成本失控

调整触发器后，要比较同一类变更的 run 数、job 数、runner total 和 required wall-clock。不能因为拆出了更多 workflow 就让一次提交重复执行相同测试；也不能为了避免漏测，把所有手动诊断和完整性能 workload 都塞进每个 PR。

事件触发、并发分组、取消策略、required check 名称和 workflow path filter 必须作为一个整体变更和验证。触发器只减少不必要的工作，不改变真正适用的质量门禁。

## 6. 测试组合与执行层级

### 6.1 按测试意图选择执行层

| 测试意图                                                                           | 首选层级                            | 必须保留真实浏览器/环境的情况                    |
| ---------------------------------------------------------------------------------- | ----------------------------------- | ------------------------------------------------ |
| 纯函数、数据转换、请求构造、序列化、状态机和 handler 语义                          | Vitest 或等价单测                   | 无                                               |
| React 组件语义、DOM 状态、事件回调和可访问名称                                     | Vitest + jsdom                      | 需要真实 layout、focus 或浏览器 API 时           |
| CSS geometry、overflow、resize、focus ring、键盘/触摸、scroll 和真实 viewport      | Playwright                          | 必须保留                                         |
| 生产 preview 路由、关键页面 landmark、真实导航、SSE/network failure 和 PWA runtime | Playwright                          | 必须保留                                         |
| SQLite current-schema 业务状态且不验证真实文件路径                                 | 受隔离的 template/in-memory fixture | 需要 migration、文件路径、权限、锁或跨进程语义时 |
| migration、archive/gzip、文件损坏、write-lock、文件读写和真实 deadline             | backend Stateful/Archive profile    | 必须保留真实文件或真实时间边界                   |

减少真实浏览器占比的原则不是把浏览器测试整体删除，而是把确定性、无需 layout 和无需浏览器 API 的断言下沉到 Vitest/jsdom，同时保留浏览器真正提供的证据。测试数量变化不等于覆盖变化；需要比较测试意图、关键分支、错误场景和用户可见行为。

### 6.2 浏览器测试下沉流程

1. 先按断言识别它验证的是语义、DOM 状态、layout、真实导航还是 runtime integration。
2. 将纯语义和状态组合迁移到 Vitest/jsdom，保持相同输入、输出、错误和状态不变。
3. 在 Playwright 中保留真实 route、landmark、viewport、几何、focus、keyboard、touch、network/SSE failure 和 PWA 行为。
4. 对迁移前后分别记录 Vitest 与 Playwright 的覆盖职责、失败诊断和执行时间；不能只以“浏览器用例少了”作为成功标准。

PR #1178 的经验是：可以把重复的 route、serialization、workload/task-detail 和模型上下文语义下沉到 Vitest/jsdom，同时保留生产 demo route、错误场景、chart geometry、mobile overflow 和维护流程的真实浏览器覆盖。PR #1182 的 frontend validation 工作则提醒我们，工作流组合和测试层级必须一起看，不能只报告一个局部 suite 的时间。

### 6.3 Stateful SQLite fixture

对 current-schema、只验证 DB 行为的测试，runner 可以一次通过真实 `ensure_schema` 生成私有 template，再用 SQLite backup API 为每个测试或进程复制隔离副本。落地前必须验证 schema/default-data parity、pooled connection visibility、双向写入和跨测试隔离。

以下测试不能因为追求速度而迁入 template/in-memory 路径：legacy migration、真实文件路径、文件权限、gzip/archive、文件损坏、write-lock、跨进程可见性和需要验证正式 deadline 的测试。fixture 优化的是重复 setup，不是删除行为边界。

PR #1192 的经验是，template/backup 可以缩短 current-schema fixture 初始化，但只有在完整 profile、隔离性和保留的真实文件回归都通过时才成立。单次局部 fixture 变快，不能直接推导整个 Stateful job 或整个 CI 变快。

## 7. 性能实验与验收方法

### 7.1 实验控制面

每个候选方案至少固定以下变量：

- 同一个 commit SHA 和相同的源码 workload。
- 相同的 GitHub runner 类型、工具链、依赖版本和 workflow 入口。
- 明确区分 cold cache、warm cache 和 cache miss；不能让候选只享受基线没有的预热。
- 相同的完整 test profile、浏览器 suite、artifact/gate 流程和失败重试规则。
- 相同的统计窗口和重复次数；单次偶然的 hosted runner 抖动不能决定拓扑。

适合的实验流程是：先记录 pristine baseline，再只改变一个变量，运行完整 workload，比较 job/DAG/runner/正确性指标，最后决定保留、继续实验或回退。手动 immutable A/B workflow 应绑定明确 SHA，并把候选结果作为诊断证据，不自动替代 required workflow。

### 7.2 验收门槛

候选方案必须同时满足：

- required wall-clock 或 critical path 有明确改善。
- runner total 不增加，或有仓库已批准的等价资源解释；不能只把时间转移到更多并行 job。
- 所有适用 required checks、test intent、coverage boundary 和 fail-closed gate 保持通过。
- 没有新增 flaky、skipped-as-success、producer failure 隐藏或 cache 污染。
- artifact、cache、log 的字节数和 retention 没有因为方案而无必要增加。
- 结果能绑定到 candidate SHA，并可由同一套命令或 run 重新观察。

性能实验中的 `baseline`、`candidate`、`workload`、`observed`、`decision` 和 `rollback` 结论应记录在对应 PR/实验资料中；本 Solution 只沉淀可复用规则，不保存每次实验的流水账。

### 7.3 无效证据

以下证据不能支持“CI 已优化”的结论：只跑本地单测、只跑一个 profile、只比较一个 step、不同 commit、不同 runner、不同缓存状态、缺失浏览器/后端 workload、producer 失败后 gate skipped、只看平均值不看失败率，或只报告墙钟而不报告 runner total。

## 8. 质量边界与防回退机制

性能改动必须同时守住四条边界：

1. **测试边界**：不删除测试意图，不放宽断言、阈值、正式 deadline 或错误场景；下沉到 Vitest 的测试仍需覆盖相同语义。
2. **运行边界**：不改变生产代码、默认 retry/backoff、SQLite 文件语义、浏览器真实交互或发布行为来制造测试速度。
3. **门禁边界**：required check、aggregate gate、producer failure 和 merge queue 结果必须 fail-closed；不能用 skipped、continue-on-error 或路径过滤隐藏失败。
4. **资源边界**：不通过无上限并发、更大 runner、重复 cache/artifact、过长 retention 或大诊断包增加 Actions 消耗。

每个 workflow topology 改动都应同步检查 quality-gates contract、required check 名称、trusted-source 自举路径和 live ruleset。当前仓库的质量门禁 Solution 已明确：上游失败要转成显式 failed gate，不能依赖后续 job 自然 skipped。

如果候选方案未同时改善关键路径和 runner total，或引入 flaky、资源争用、失败隐藏或消耗增加，应保留诊断证据并回退到上一个已验证拓扑。回退优先恢复 workflow 依赖、cache namespace、worker/profile 和 artifact 交接边界，不通过临时放宽测试来“修绿”。

## 9. 落地与持续治理

推荐按以下顺序推进，每一步都保留可回退的候选边界：

1. **建立基线**：记录 run class、job DAG、wall-clock、critical path、runner total、top offenders、artifact/cache 字节和正确性结果。
2. **消除重复**：合并重复依赖安装、浏览器安装、编译、fixture 初始化和产物生成；先验证 artifact 交接不会增加传输关键路径。
3. **优化 DAG**：拆分真实独立的 job，使用 producer/consumer/gate，限制共享 server、SQLite 和 runner CPU 的并发。
4. **调整测试层级**：将确定性语义下沉到 Vitest/jsdom；保留 layout、真实导航、runtime、文件和锁行为的真实环境覆盖。
5. **收敛触发器**：区分 PR、merge queue、Main 和手动诊断；用路径和变更分类减少不适用 workload，同时保留 required check 可见性。
6. **持续观察**：对新拓扑按固定维度比较冷/热、单 job、关键路径、runner total、失败/重试和 Actions 消耗；低于收益门槛时回退。

长期维护时，workflow 文件、contract checker、required checks、Solution 和 Spec 必须形成引用关系。每次 topology、触发器或质量门禁变更都应先检查现有 Solution 是否需要 refresh；不要把一次 PR 的时间线复制进 Solution，也不要把历史局部数据当成当前 SLA。

## 10. 当前与过往实现映射

| 当前或历史表面              | 已有事实/经验                                                                                                                                         | 可复用原则                                                                                            |
| --------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `CI PR`                     | 同时覆盖 `pull_request` 和 `merge_group`，包含 frontend、Storybook、docs/demo、E2E producer/gate、backend profiles、smoke artifact 和 aggregate gates | PR candidate、merge queue、producer/consumer 和 required gate 必须有清晰边界                          |
| `CI Main`                   | 主干完整验证，使用不取消的 main concurrency，并保留 Stateful SQLite 两个 shard 和 aggregate check                                                     | 主干完成路径不能被 candidate cancellation 破坏；分片必须由 fail-closed aggregate 汇总                 |
| `Observability Performance` | 通过 `workflow_dispatch` 运行受控性能 workload，concurrency 不取消，诊断 artifact 有独立 retention                                                    | 性能诊断按需触发，绑定明确 SHA，不默认进入每个 PR                                                     |
| backend archive/profile     | PR #1177 减少重复 archive setup；现有 backend Solution 将 registry/git cache、source-key target、nextest archive 和资源 profile 分开                  | 先复用不可变构建输入，再由消费者执行各自 profile；cache 复用必须用 critical path 和 runner total 验收 |
| Stateful SQLite             | 两个 hash shard、六线程 profile、template/backup fixture 和真实文件回归并存                                                                           | 优化重复 setup，不把 migration、file I/O、lock 和 archive 语义改成廉价 fixture                        |
| 前端测试                    | PR #1178 将重复语义覆盖下沉到 Vitest/jsdom，保留真实 route、layout、interaction、mobile 和 runtime；PR #1182 继续收敛 frontend validation 组合        | 真实浏览器只承担浏览器能证明的行为，语义测试在更便宜的层级完成                                        |
| quality gates               | `main` branch protection 要求完整 required checks；producer failure 必须显式传播到 gate                                                               | 任何并行、按需触发或 producer 拆分都不能把失败变成 skipped 或 informational                           |
| 历史 A/B 运行               | 旧实验表明局部 fixture、编译复用或单个前端 suite 变快，不等于 whole-CI 改善                                                                           | 同一 SHA、同一 runner、完整 workload、冷/热两轮和 runner total 才能支持保留决策                       |

这些历史条目用于解释设计来源和验证边界，不是对当前每次 run 的时长承诺。新改动应以当前 workflow、live required checks 和绑定到当前 SHA 的实验结果为准。

## References

- [CI PR workflow](../../../.github/workflows/ci-pr.yml)
- [CI Main workflow](../../../.github/workflows/ci-main.yml)
- [Observability Performance workflow](../../../.github/workflows/ci-observability-performance.yml)
- [Backend test runtime feedback loop](./rust-backend-test-runtime-feedback-loop.md)
- [Main branch protection and quality gates](../workflow/main-branch-protection-quality-gates.md)
- [Backend test resource profiles Spec](../../specs/q7yt7-backend-test-resource-profiles/SPEC.md)
- [Backend test runner](../../../.github/scripts/run-backend-tests.sh)
- [Quality-gates contract checks](../../../.github/scripts/test-quality-gates-contract.sh)
- [PR #1177: reduce duplicate archive setup](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1177)
- [PR #1178: migrate redundant browser coverage](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1178)
- [PR #1182: streamline frontend validation](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1182)
- [PR #1192: optimize SQLite fixture setup](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1192)
- [GitHub Actions billing and usage](https://docs.github.com/en/actions/concepts/billing-and-usage)
- [GitHub Actions concurrency](https://docs.github.com/en/actions/concepts/workflows-and-actions/concurrency)
- [Store and share data with workflow artifacts](https://docs.github.com/en/actions/tutorials/store-and-share-data)
- [Remove workflow artifacts](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/remove-workflow-artifacts)
