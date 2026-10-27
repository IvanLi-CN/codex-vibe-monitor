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

GitHub Actions CI 的慢通常不是单个测试函数的问题，而是一个由触发器、任务图、runner 准备、依赖安装、编译、fixture、测试、浏览器、产物传递和质量门禁共同组成的系统问题。一次 PR run 可能包含多个本来可以并行的工作，也可能在不同 job 中重复执行同一份准备工作。只看某个测试命令的耗时，无法回答两个关键问题：

1. 开发者从提交到得到 required 结果实际等了多久？
2. 这次变更总共消耗了多少 runner 工作，是否只是把时间搬到了更多 job？

本 Solution 的用途，是把这两个问题转换成可复用的设计、实验和验收规则。它适用于同时包含前端、后端、真实浏览器、构建产物、数据库 fixture 和质量门禁的仓库，也适用于只有其中一部分能力的仓库。文档中的通用策略可以单独采用，但必须先经过当前仓库的 workflow、required checks 和测试意图核对。

### 1.1 需要解决的问题形态

当出现以下任一现象时，应先按本 Solution 进行定位，而不是直接增加 runner 或删除测试：

| 现象                                        | 可能的系统问题                                        | 先查什么                           |
| ------------------------------------------- | ----------------------------------------------------- | ---------------------------------- |
| 最慢 job 只有一个，但 required 结果仍然很慢 | 关键路径被串行依赖、产物等待或门禁等待拉长            | DAG、needs、gate 结束时间          |
| 单个测试变快，整体 run 没变快               | 测试不在关键路径，或 setup/下载占主导                 | job/step 时间分解                  |
| 并行后墙钟变短，但 Actions 消耗上升         | 工作被复制到更多 runner，或缓存/产物传输重复          | runner total、job 数、字节数       |
| 只有 CI 偶尔超时或 flaky                    | 资源争用、过窄 deadline、fixture 共享或浏览器并发过高 | 失败日志、线程/worker、runner 负载 |
| 文档或小改动也触发完整套件                  | 触发器没有按 run class 或变更域区分                   | paths、事件、no-op gate            |
| producer 失败后下游显示 skipped             | gate 没有 fail-closed，失败被工作流语义隐藏           | needs、if、aggregate gate          |

这些现象可以同时存在。优化顺序应先保证失败可见和测试意图不变，再处理关键路径，最后处理 runner 总执行时间和传输开销。

### 1.2 目标

| 目标                     | 可观察结果                                               | 不能用什么替代                           |
| ------------------------ | -------------------------------------------------------- | ---------------------------------------- |
| 降低 required wall-clock | 从适用 required job 开始到最终 gate 结束的时间缩短       | 不能只报告一个 unit-test step            |
| 降低 critical path       | DAG 上最长依赖链的有效工作减少                           | 不能把串行依赖简单复制成更多并行 job     |
| 降低 runner total        | 同一变更所需的 job 执行秒数不增加，重复工作减少          | 不能用隐藏失败、跳过或扩大缓存来制造数字 |
| 保持质量边界             | 测试意图、覆盖边界、质量门禁和生产行为不变               | 不能把测试迁移当作删除测试               |
| 让结论可复现             | 实验绑定 SHA、run class、runner、缓存状态和完整 workload | 不能用不同提交或不完整套件作 A/B         |

本 Solution 的第一目标是性能，第二目标是可解释性。任何“更快”的结果都必须说明快在哪里、少做了什么重复工作、是否仍然证明了原来的行为，以及 runner 总工作是否保持不增加。

### 1.3 硬约束与非目标

以下约束贯穿全文：

- 不删除测试意图，不放宽质量门禁，不修改生产行为。
- 不把 GitHub Actions 账单降低写成独立目标；Actions 消耗只允许不增加。
- 不通过增加无必要 job、扩大 runner、延长 artifact retention、上传大附件或重复运行来换取墙钟缩短。
- 不把真实浏览器、真实文件、migration、锁竞争和跨进程行为迁移到无法证明这些行为的廉价 fixture。
- 不把一次局部实验、一次 hosted runner 抖动或一次重试结果写成全量 CI 的永久结论。

本 Solution 不规定所有仓库必须使用相同的 job 名称、runner 类型、分片数量或测试工具。它规定的是判断顺序和证据边界：先确认行为，再确认适用性，再优化关键路径，最后确认 runner 总工作和传输开销。

### 1.4 每次优化应交付什么

一次可接受的 CI 性能改动至少应能回答以下问题：

1. 基线和候选分别绑定到哪些 commit SHA、事件和 workflow run？
2. 改动了哪一条依赖、哪一类测试层级、哪一个 fixture 或哪个触发器？
3. 关键路径减少了哪段工作？哪些工作只是被移动到了并行 job？
4. runner total、job 数、artifact/cache 字节和 retention 是否增加？
5. 原有测试意图和质量门禁由哪些候选 job 继续证明？
6. 如果候选失败或回退，恢复哪个 workflow、cache namespace、fixture 或测试层级？

无法回答这些问题时，结果只能作为诊断信息，不能作为已完成的性能优化。

## 2. CI 性能指标与成本不增加约束

### 2.1 指标模型

为了避免“墙钟”和“总用时”混用，使用以下定义：

| 符号/指标           | 定义                                                          | 注意事项                                        |
| ------------------- | ------------------------------------------------------------- | ----------------------------------------------- |
| required wall-clock | 适用 required job 中最早启动时间到最终 required gate 完成时间 | 反映开发者等待，不等于所有 job 时长之和         |
| critical path       | 按实际 needs 依赖计算，从触发到 required gate 的最长依赖链    | 并行 job 只有在不共享依赖且不争用资源时才有收益 |
| job wall time       | 单个 job 的 startedAt 到 completedAt                          | 包含该 job 自己的 setup、下载、测试和上传       |
| runner total        | 同一 workload 的所有适用 job wall time 之和                   | 用来识别“墙钟变短但工作复制了”的方案            |
| queue/setup         | 排队、runner 分配、checkout、工具链、依赖和缓存恢复           | 不能归入测试本身，否则会选错优化点              |
| test execution      | 实际测试或浏览器命令的执行区间                                | 仍需与 fixture、server、下载和报告分开          |
| transfer overhead   | cache restore/save、artifact upload/download、解压和聚合      | 复用方案必须证明这部分没有抵消收益              |
| correctness outcome | pass、fail、cancelled、skipped、retry 和 gate 结果            | 速度不能来自隐藏失败                            |

可用以下关系理解优化方向：

```
required_wall_clock = required_gate_end - first_applicable_required_start
critical_path = max(sum(effective_work_on_each_dependency_chain))
runner_total = sum(job_end - job_start for all applicable jobs)
```

上式不是 GitHub 账单计算器。它是对同一 workload 做方案比较的统一口径。实际计费还可能受 runner 类型、计费粒度和组织配置影响，因此本 Solution 不预设价格或账单金额。

### 2.2 基线记录表

开始实验前，至少记录一条完整 baseline。建议字段如下：

| 字段                 | 示例值形态                               | 作用                        |
| -------------------- | ---------------------------------------- | --------------------------- |
| candidate SHA        | 完整 commit SHA                          | 防止不同源码被误作 A/B      |
| run class            | PR、merge queue、main、manual diagnostic | 区分触发语义和适用 workload |
| workflow/run/attempt | workflow 名、run ID、attempt             | 追溯完整 DAG 和重试         |
| runner identity      | hosted runner label 或 self-hosted pool  | 排除硬件差异                |
| toolchain            | Rust/Bun/Node/浏览器版本                 | 排除工具链变化              |
| cache state          | cold、warm、miss、partial hit            | 防止候选拥有基线没有的预热  |
| job/step timing      | startedAt、completedAt、关键 step 区间   | 定位关键路径和重复准备      |
| artifact/cache bytes | 上传、下载、保存和恢复大小               | 检查传输是否增加            |
| retention            | artifact/log retention 配置              | 检查长期保存是否扩大        |
| correctness          | required checks、flaky、retry、skipped   | 证明质量边界未被削弱        |

基线应包含完整适用 workload。只记录一次 Vitest、一个 backend profile 或一个 producer step，不能作为 whole-CI baseline。

### 2.3 Actions 消耗不增加的验收规则

将候选与基线对比时，使用以下硬门禁：

| 检查项        | 通过条件                                            | 失败处理                           |
| ------------- | --------------------------------------------------- | ---------------------------------- |
| run 数        | 同一变更没有因为新触发器重复启动等价 run            | 回退触发器或合并重复入口           |
| job 数        | 没有新增无功能价值的 job；新增 job 必须替代重复工作 | 不接受仅为并行而复制工作           |
| runner total  | 同一完整 workload 不增加                            | 即使墙钟变短，也拒绝候选           |
| cache 工作    | restore/save 的目录、次数和字节没有无必要增加       | 收窄 key、目录或 producer 边界     |
| artifact 工作 | 只交接 consumer 必需的最小文件                      | 移除工作区、依赖目录和重复报告     |
| retention     | 只保留诊断、交接和审计所需的最短期限                | 缩短 retention，不用长期保存换便利 |
| 正确性        | required gate、失败传播和覆盖边界保持等价           | 进入回退或重新设计                 |

“不增加”要求比较同等 workload 和同等证据。如果候选减少了 PR workload，却把同样工作全部搬到 main 或 manual run，不能把它描述为整体消耗不增加；必须说明 run class 的变化和完整生命周期。

### 2.4 Artifact、Cache 与日志的最小化

三类对象的职责不同：

| 对象                    | 适合保存                                   | 不应保存                                                | 处理原则                                   |
| ----------------------- | ------------------------------------------ | ------------------------------------------------------- | ------------------------------------------ |
| cache                   | 可由同一 source key 重建的依赖和编译输入   | 每次 run 的测试结果、完整工作区、失败诊断               | key 精确、目录最小、失败不保存半成品       |
| artifact                | 下游 job 必须消费的不可变交接物和必要报告  | node_modules、完整 target、浏览器安装目录、无消费者日志 | producer 完整成功后上传，consumer 用完即止 |
| log/diagnostic artifact | 失败定位所需的 trace、截图、摘要和关键日志 | 每次成功 run 的完整环境快照                             | 默认失败时上传，保留期按排障需要最小化     |

附件体积和保留期必须尽量小。上传前先列出每个文件的消费者、生命周期和必要字段；没有消费者、只为“以后可能有用”保存的文件，应删除或不上传。压缩不能成为无边界收集大目录的理由：压缩前后的传输、解压和保留成本都要计入实验。

### 2.5 读数解释

不要单独解释一个数字：

- wall-clock 下降、runner total 上升：通常是并行复制或资源扩大，候选不通过硬约束。
- runner total 下降、wall-clock 不变：工作可能不在关键路径，或 gate/最长 job 没变；可以作为二次收益，但不能宣称开发者等待变快。
- job 变快、artifact transfer 变慢：需要比较完整 job wall time 和关键路径，而不是只看测试命令。
- 平均时间变快、失败率或重试上升：性能收益被可靠性损失抵消。
- 只有 warm cache 变快：先确认 cache miss、冷 runner 和首个 PR 的真实体验。

## 3. CI 瓶颈识别方法

### 3.1 从 run 到 step 的定位流程

每次调查按固定顺序进行，避免一上来改测试：

1. **确定 run class**：确认是 PR、merge queue、main 还是手动诊断；不同入口可能有不同 job。
2. **固定 candidate**：记录 base SHA、head SHA、runner、工具链和缓存状态。
3. **画出 DAG**：列出 workflow job、needs、条件、producer、consumer 和 aggregate gate。
4. **找关键路径**：从适用 required job 的最早开始时间向最终 gate 回溯，标出没有 slack 的链路。
5. **拆 step 时间**：对关键 job 分解 checkout、toolchain、cache、compile、fixture、test、browser、upload 和 cleanup。
6. **做重复工作清单**：找出多个 job 都在安装、编译、生成 fixture 或上传同类文件的地方。
7. **做资源清单**：记录线程、worker、连接池、共享 server、SQLite 文件和 runner CPU 的关系。
8. **确认失败语义**：检查失败是否显式传播，还是被 skipped、continue-on-error 或不适用条件隐藏。
9. **只选一个变量实验**：先改变一个依赖、缓存、分片、fixture 或测试层级，再重新测量。

这个流程把“CI 慢”拆成可行动的假设。例如，“Stateful job 慢”可能是编译慢、schema setup 慢、SQLite lock 等待、测试线程过高或等待 aggregate gate；它们的修复方向完全不同。

### 3.2 症状到证据的映射

| 症状                        | 需要的证据                              | 可能的根因                            | 候选方向                        |
| --------------------------- | --------------------------------------- | ------------------------------------- | ------------------------------- |
| 所有 job 都在等待同一个 job | job startedAt、needs 图                 | 过宽的 producer 或错误串行            | 拆 producer、并行独立 consumer  |
| compile 占 job 大部分       | compile step、cache key、archive 大小   | 多 job 重编译或 cache miss            | 精确 source key、不可变 archive |
| test 很短但 job 很长        | setup/upload/download 时间              | 准备和交接占主导                      | 缩小 artifact、合并准备工作     |
| Stateful 线程越多越慢       | profile 线程矩阵、runner CPU、lock 日志 | oversubscription 或 SQLite contention | 限制线程、按资源拆 profile      |
| Playwright suite 不稳定     | worker 数、server readiness、trace      | 浏览器和 server 争用                  | 固定 worker、减少重复浏览器启动 |
| docs-only 也跑完整套件      | event/path/run 数                       | 触发范围过宽                          | 变更分类和可见 no-op gate       |
| gate 为绿色但上游 skipped   | needs result、if、gate 逻辑             | 失败传播不闭合                        | always aggregate gate           |
| 重试后变绿                  | 首次和重试日志、失败类型                | flaky 或 runner 瞬态压力              | 先分类，不能把 retry 当加速     |

只有同时拥有“症状”和“证据”才能建立根因。历史经验可以用来形成假设，但不能替代当前 candidate 的测量。

### 3.3 关键路径与总工作量

用以下简化图说明为什么要同时看两类指标：

```
          ┌─ frontend ─────┐
event ────┼─ lint ──────────┼── gate
          ├─ backend build ─┤
          └─ backend tests ──┘
```

如果 frontend、lint 和 backend 独立，required wall-clock 接近最长分支加 gate，而 runner total 是三条分支之和。把一条 10 分钟工作拆成两条 5 分钟工作，可能缩短墙钟，但如果两条都重复做同样的编译和 fixture，runner total 反而增加。正确的并行化应同时减少依赖等待和重复工作。

调查时应分别回答：

- 哪些 job 没有真实数据依赖，却被 needs 串行？
- 哪些 job 有真实共同输入，适合由 producer 一次生成？
- 哪些 job 共享 runner 资源，不能简单增加 worker？
- 哪个 gate 决定 required 结束时间？
- 哪些非 required job 仍然占用大量 runner，却没有被当前变更使用？

### 3.4 当前仓库历史数据的正确用途

历史 run/PR 的价值不是提供一个“目标秒数”，而是把抽象规则绑定到已经发生过的设计选择。应以“事实、结论、使用边界”三列保存：

| 案例                                                    | 事实                                                                                                      | 结论与使用边界                                                                    |
| ------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| backend runtime feedback loop 中的 Main run 31706131099 | Stateful SQLite 记录了 617s wall，其中 compile 143s、test execution 404s，backend-related jobs 合计 1257s | 证明必须把 compile、test、profile 和 DAG 分开看；不能当作当前 SLA                 |
| 同一 Solution 中的 PR run 31825458818                   | Lint、Lightweight/Stateful/Archive 和 Build Artifacts 有各自独立的时间读数                                | 证明 setup、构建产物和测试 profile 都可能成为瓶颈；不能推导所有 PR 都有相同耗时   |
| PR 1177                                                 | 处理重复 archive setup                                                                                    | 作为 producer/consumer 复用的具体案例；需重新验证当前 workflow 是否仍保留同类重复 |
| PR 1178                                                 | 将重复的前端语义覆盖下沉到 Vitest/jsdom，并保留真实浏览器边界                                             | 作为测试意图分层案例；不能据此删除 layout、navigation 或 runtime 覆盖             |
| PR 1182                                                 | 收敛 frontend validation 组合                                                                             | 作为 workflow 组合与测试层级联动案例；不能只比较单个 suite                        |
| PR 1192                                                 | 优化 Stateful SQLite fixture setup                                                                        | 作为 template/backup fixture 案例；不能替代 migration、文件、锁和 archive 回归    |

因此，历史 ID 应出现在案例映射或引用中，而不应孤立地堆在正文里。没有“它改变了什么决策”的历史数字，就不应保留。

### 3.5 最低诊断工具集

不依赖特定第三方平台时，也应能从以下事实开始：

1. workflow 源码：事件、concurrency、job、needs、if、artifact 和 retention。
2. run 元数据：run class、SHA、run/attempt、job startedAt/completedAt、结论。
3. job 日志：关键 step 的 setup、compile、fixture、test、upload 和失败信息。
4. 变更历史：同类 PR、同类失败、最近一次可比的通过 run。
5. 测试脚本与 profile：实际 filter、线程、worker、fixture mode 和 retry。

平台命令只是读取入口，不能替代解释。每个结果都应回到“关键路径、重复工作、资源争用、失败传播或触发范围”中的一个明确假设。

## 4. Workflow 内部设计

### 4.1 推荐的 DAG 形状

通用目标形状如下：

```
event
  │
  ▼
change classification
  │
  ├───────────────┬────────────────┬─────────────────┐
  ▼               ▼                ▼                 ▼
lint/tooling   frontend         docs/demo       backend producer
                                                 │
                              ┌──────────────────┼─────────────────┐
                              ▼                  ▼                 ▼
                         lightweight       stateful shards      archive/file
                              └──────────────────┬─────────────────┘
                                                 ▼
                                         aggregate gate
                                                 │
                                                 ▼
                                          required check
```

图中的连线只表示真实依赖。没有共享输入的工作应并行；共享不可变输入的工作由 producer 生成一次，再由 consumer 使用；最终 gate 必须显式汇总所有适用结果。

### 4.2 四类职责

| 角色       | 必须做什么                              | 不应做什么                                 | 成功条件                 |
| ---------- | --------------------------------------- | ------------------------------------------ | ------------------------ |
| Classifier | 确定 run class、变更域和适用 workload   | 靠隐式 path filter 让 required check 消失  | 适用性可观察且可追溯     |
| Producer   | 在固定 SHA 上生成一次完整、不可变的输入 | 保存半成品、让 consumer 再编译             | 输出完整、校验过、可下载 |
| Consumer   | 下载必要输入并执行自己的测试/profile    | 重复 producer 的构建和 fixture             | 只消费所需文件，失败显式 |
| Gate       | 汇总 producer 和所有 consumer 的结果    | 依赖 skipped 或 continue-on-error 假设成功 | 上游失败时 gate 明确失败 |

Producer/consumer 不是为了增加抽象层，而是为了消除真实重复工作。若共享输入很小、下载解压比重建更慢，直接在 consumer 中生成可能更好；该决定必须用完整 job wall time 和 runner total 验证。

### 4.3 Producer 合同

Producer 需要明确以下合同：

- 输入绑定到完整 commit SHA、工具链、lockfile 和 profile。
- 输出目录有清单，说明每个文件的 consumer 和用途。
- 输出生成完成后再上传；构建、打包或校验失败时不得上传可被误用的半成品。
- consumer 下载后能验证 archive、binary、bundle 或 fixture 的身份。
- artifact 名称包含 run/attempt 或候选身份，避免不同 run 覆盖。
- retention 只覆盖下游消费和短期诊断所需的时间。
- producer 的失败必须成为 aggregate gate 的失败，而不是让消费者自然 skipped。

对于大型 target、nextest archive 或 smoke binary，producer 应优先输出 consumer 真正需要的最小集合。把整个工作区上传给下游，通常只是把编译重复转化成传输和解压重复。

### 4.4 Consumer 合同

Consumer 应遵守以下边界：

1. 先验证 producer 输出存在、身份和完整性。
2. 只安装或恢复自己需要的工具和依赖。
3. 不重新编译 producer 已交付的二进制，除非实验明确比较“下载 vs 重建”。
4. 使用自己的测试 profile、filter、线程和 worker 边界。
5. 上传只对该 consumer 有意义的报告；成功路径不上传大诊断包。
6. 以测试退出码作为自身结果，不能用“没有运行测试”报告成功。

如果 consumer 必须修改 producer 输出，应把修改后的身份和生命周期写入合同；不可变 artifact 不应在多个 consumer 之间共享可写目录。

### 4.5 Aggregate gate 与失败传播

每个 aggregate gate 都应验证三件事：

| 检查            | 说明                                                                          |
| --------------- | ----------------------------------------------------------------------------- |
| producer result | producer 必须明确 success，不能只有 artifact 存在                             |
| consumer result | 所有适用 consumer 必须 success；失败、取消和缺失都不能被忽略                  |
| applicability   | 被分类为不适用的 job 必须有显式理由和可见结果，不得与 skipped-as-success 混淆 |

典型伪代码如下，表示设计意图，不是要求直接复制到当前 workflow：

```yaml
gate:
  needs: [producer, consumer-a, consumer-b]
  if: always()
  steps:
    - verify producer result is success
    - verify every applicable consumer result is success
    - fail when any required result is failed, cancelled, or missing
```

如果一个 job 因上游失败而 skipped，gate 仍必须通过 needs 结果识别上游失败。always 只保证 gate 有机会观察结果，不代表 gate 可以无条件成功。

### 4.6 并行、分片与资源 profile

并行拆分的收益来自两点：减少依赖等待，以及让不同资源类型同时工作。它的成本也来自两点：每个 job 都有 setup/下载/清理开销，以及共享 runner 或服务可能发生争用。

分片设计应记录：

- 分片键是什么，是否稳定、可复现、能避免严重倾斜。
- 每片包含哪些测试意图，是否有跨片共享状态。
- 每片需要多少线程、连接、临时空间和数据库文件。
- aggregate gate 如何知道分片完整，如何处理缺片和取消。
- 分片数增加后，setup、artifact、runner total 和失败重试是否增加。

Stateful SQLite、Archive/File I/O 和 Playwright 不应只按测试数量平均分片。一个包含文件、锁或真实 deadline 的测试可能比多个纯语义测试更重；应优先按资源 profile 和历史 wall time 调整。

### 4.7 Cache 与 Artifact 的决策表

| 问题                                    | 更适合 cache | 更适合 artifact/producer |
| --------------------------------------- | ------------ | ------------------------ |
| 内容可以由 source key 重新生成吗？      | 是           | 通常不是                 |
| 下游需要同一 run 的精确输出吗？         | 否           | 是                       |
| 内容是否包含当前 run 的测试结果或诊断？ | 否           | 是                       |
| 是否需要跨 job 保持不可变身份？         | 可选         | 必须                     |
| 是否可能被多个 job 并发写入？           | 避免         | 只读消费                 |

cache key 至少应考虑 lockfile、manifest、工具链、profile 和源码 fingerprint。缓存 restore/save 的时间和字节数必须和省下的 compile/setup 时间一起测量；“命中”本身不是性能收益。

### 4.8 并发取消与失败传播

普通 PR candidate 可以按 PR 编号分组，新的 candidate 取消同一 PR 的旧 candidate，避免过期 run 继续占用 runner。merge queue、main、发布完成路径和需要保存完整诊断的 run 通常不应使用相同的取消语义。

设计 concurrency 时要明确：

- 哪些 run 可以取消，哪些 run 一旦开始就必须完成。
- 取消后 required check 如何显示，是否会留下 pending。
- merge queue 的最终候选是否会被 PR candidate 的取消规则误伤。
- manual diagnostic 是否按 SHA 隔离，避免两个实验互相取消。

取消旧 run 只减少过期工作；它不能作为隐藏失败或降低质量门禁的手段。

### 4.9 Workflow 设计检查表

提交 workflow topology 改动前逐项检查：

- 独立 job 是否已去除不必要的 needs？
- 共享构建或测试输入是否只有一个 producer？
- consumer 是否仍在重复编译或生成 fixture？
- producer 半成品是否不会被消费？
- gate 是否使用真实结果并 fail-closed？
- path filter 是否保留 required check 的可见结果？
- concurrency 是否区分 PR、merge queue、main 和 manual？
- artifact/cache 的目录、大小和 retention 是否有明确理由？
- runner total 是否有同 workload 的 baseline？

## 5. 按需触发策略

### 5.1 Run class

按需触发不是“尽可能少跑测试”，而是让每类事件执行它真正需要的质量边界：

| run class              | 入口                              | 主要问题                       | 适合的 workload                                  | 取消语义                       |
| ---------------------- | --------------------------------- | ------------------------------ | ------------------------------------------------ | ------------------------------ |
| PR candidate           | pull_request                      | 变更是否可被 review 和合并     | 变更相关 required checks、快速反馈和必要集成测试 | 同一 PR 的旧候选通常可取消     |
| merge queue            | merge_group                       | 最终合并候选在目标基线上的结果 | 与保护规则匹配的完整 required checks             | 不因普通 PR candidate 规则取消 |
| Main                   | push 到 main                      | 主干完整性和后续发布输入       | 主干验证、发布候选和完整质量门禁                 | 通常不取消                     |
| Performance/diagnostic | workflow_dispatch 或明确 schedule | 性能、资源和 A/B 诊断          | 明确 SHA 的完整 workload、对照和诊断产物         | 不取消，保留实验身份           |

当前仓库的 CI PR 处理 pull_request 和 merge_group，CI Main 处理主干 push，Observability Performance 提供手动性能入口。三者不应互相替代：手动诊断结果不能直接成为 PR required 结果，PR 快速反馈也不能替代主干完整验证。

### 5.2 变更域与 workload 矩阵

以下矩阵是选择原则，具体 job 仍要以仓库的质量合同为准：

| 变更域                               | PR candidate                                    | merge queue                  | main               | manual diagnostic       |
| ------------------------------------ | ----------------------------------------------- | ---------------------------- | ------------------ | ----------------------- |
| 仅文档或不影响构建输入的说明         | 文档/仓库门禁和可见 no-op gate                  | 保护规则要求的检查           | 主干既有验证       | 不自动触发              |
| 前端语义、组件和状态                 | Vitest、类型/构建、受影响的浏览器边界           | 与保护规则一致的完整前端边界 | 主干前端验证       | 可按需做浏览器性能      |
| layout、viewport、真实导航、PWA、SSE | Vitest 辅助 + Playwright 真实边界               | 完整真实浏览器边界           | 主干真实浏览器边界 | 绑定 SHA 的性能/诊断    |
| backend source 或 test profile       | backend archive/producer、适用 profiles 和 gate | 完整 required backend        | 主干完整 profiles  | 线程、fixture、资源 A/B |
| workflow、质量门禁、共享 fixture     | 不能走窄路径，需验证相关完整合同                | 完整保护路径                 | 主干和发布输入路径 | 可补充拓扑实验          |
| 纯性能调查                           | 不默认加入每个 PR                               | 不默认加入                   | 按主干策略         | 手动入口，保留完整证据  |

这张表的目的，是在触发器设计时明确“谁证明什么”。它不是允许直接跳过 required check 的清单。

### 5.3 变更分类决策顺序

建议按以下顺序分类：

1. 是否修改 workflow、质量门禁、构建输入、测试脚本或共享 fixture？如果是，至少进入相关完整验证。
2. 是否修改真实浏览器、文件、数据库、migration、锁或跨进程边界？如果是，保留对应真实环境测试。
3. 是否只修改一个不会影响运行时的文档或说明路径？如果是，可减少不适用的昂贵 workload，但 required check 必须仍然可见。
4. 是否无法证明变更域？如果无法证明，选择更宽的 workload，不以“看起来不相关”为依据缩减测试。

变更分类要有可审计输出，例如分类名称、触发的 workload、跳过的 workload 和跳过理由。没有理由的 skipped 不能作为按需优化。

### 5.4 Path filter 与可见 gate

GitHub Actions 的 path filter 可以避免不适用 job 启动，但 required check 的可见性必须单独验证：

- required workflow 被 path filter 完全跳过时，分支保护可能看到缺失或 pending。
- 一个 no-op job 可以报告“不适用”，但它必须是显式、可审计、不会把真正失败伪装成成功的合同。
- workflow、测试合同、共享构建输入和质量门禁变更通常不能使用窄路径。
- merge queue 的上下文可能与普通 PR 不同，不能只在 pull_request 上验证路径行为。

每次 path filter 改动都应使用代表性变更矩阵验证：文档-only、前端-only、backend-only、workflow-only、跨域变更和 merge queue。

### 5.5 防止重复触发

一个提交可能同时触发多个入口。调整触发器后，应统计：

- 相同 SHA 是否有多个等价 workflow run。
- label/metadata 改动是否产生重复的 CI。
- PR candidate 被取消后是否仍有残留 producer、artifact 或 pending check。
- merge queue 是否重新执行了必要检查，而不是复用不匹配的 PR 结果。
- manual diagnostic 是否明确绑定 SHA，没有被自动事件误触发。

减少重复触发的验收不是“run 数越少越好”，而是适用 workload 只执行一次，必要的质量边界仍然执行，并且 required 结果不会因为减少 run 而缺失。

### 5.6 触发器改动的验收

触发器或 concurrency 变更至少需要一组事实：

| 场景              | 应观察                                             |
| ----------------- | -------------------------------------------------- |
| 文档-only         | 不启动不适用的昂贵 workload，required check 仍可见 |
| 前端-only         | 语义、构建和真实浏览器边界按矩阵执行               |
| backend-only      | producer、profiles、shards 和 aggregate gate 完整  |
| workflow-only     | 不走窄路径，质量合同自检完整                       |
| 同一 PR 连续 push | 旧 candidate 可按规则取消，新 candidate 完整       |
| merge queue       | 不被 PR candidate concurrency 错误取消             |
| main push         | 主干验证和发布输入不因 PR 取消语义丢失             |

## 6. 测试组合与执行层级

### 6.1 先按测试意图选层级

“使用真实浏览器”不是测试质量的同义词。应先识别断言需要证明的行为，再选择最便宜且足够真实的层级：

| 测试意图                                           | 首选层级                           | 真实浏览器/真实环境不可替代的证据  |
| -------------------------------------------------- | ---------------------------------- | ---------------------------------- |
| 纯函数、数据转换、序列化、请求构造                 | Vitest 单测                        | 无                                 |
| reducer、状态机、handler 分支、错误映射            | Vitest 单测                        | 无                                 |
| React 组件状态、事件回调、DOM 语义、可访问名称     | Vitest + jsdom                     | 真实 layout、focus 或浏览器 API    |
| 组件组合和路由数据装配，但不验证 layout            | Vitest + jsdom 或 mock boundary    | 真实导航、网络失败、runtime 初始化 |
| CSS geometry、overflow、resize、scroll、focus ring | Playwright                         | 必须保留 viewport 和真实浏览器     |
| 真实 route、landmark、keyboard/touch、PWA runtime  | Playwright                         | 必须保留                           |
| SSE、network failure、真实 server readiness        | Playwright 或集成环境              | 必须保留真实网络/服务边界          |
| current-schema DB 行为，不验证文件路径和迁移       | template/backup 或隔离内存 fixture | 真实文件、连接池和跨进程语义       |
| migration、文件权限、损坏文件、archive/gzip        | Stateful/Archive profile           | 必须保留真实文件                   |
| SQLite lock、retention/backfill、deadline 压力     | Stateful/Archive profile           | 必须保留真实竞争和时间边界         |

降低真实浏览器占比的正确表达是“让浏览器只证明浏览器能证明的行为”，而不是“把 Playwright 数量降到最低”。如果一个 Playwright 用例同时包含语义断言和 layout 断言，可以拆成 Vitest 语义部分加一个更小的 Playwright 几何部分。

### 6.2 Playwright 下沉决策树

对每个候选用例逐项判断：

1. 断言是否依赖真实 viewport、CSS 计算、滚动、focus、键盘、触摸或 pointer 行为？
2. 是否依赖真实导航、server readiness、SSE、网络失败、Service Worker 或 PWA runtime？
3. 是否只是验证组件给定输入下的文本、状态、事件和错误分支？
4. 是否需要浏览器实现而 jsdom 无法提供或会产生不同语义？

处理规则：

- 只满足第 3 项时，优先迁移到 Vitest/jsdom。
- 满足第 1 或第 2 项时，保留 Playwright，并尽量把纯语义重复断言拆出。
- 第 4 项不确定时，先保留浏览器测试，建立对照后再迁移。
- 迁移后必须保留原行为的输入、输出、错误、边界和覆盖职责；不能只把 assertion 数量搬到另一个文件。

### 6.3 浏览器迁移的等价性证据

每次下沉都应有一张简短映射：

| 原浏览器用例证明           | 新 Vitest 用例证明                         | 仍保留的 Playwright 证据           | 不变的边界                     |
| -------------------------- | ------------------------------------------ | ---------------------------------- | ------------------------------ |
| 给定数据显示正确文本和状态 | 组件/handler 在同一输入下返回相同 DOM 语义 | 同一路由至少保留一个真实渲染 smoke | 输入、错误和状态转换           |
| route 数据加载后的任务列表 | mock network response 下的状态处理         | 真实 route 和失败网络路径          | loading、empty、error、success |
| chart 数据格式化           | formatter 和 tooltip model                 | 真实 canvas/SVG geometry 与 resize | 数值、单位、边界               |
| mobile overflow 行为       | 无法完全下沉                               | 保留真实 viewport、scroll 和 touch | viewport、断点、可操作性       |

迁移完成后要比较的不只是时间：

- Vitest 是否覆盖了原来由浏览器承载的语义分支？
- Playwright 是否仍覆盖真实 route、布局和 runtime？
- 失败时能否定位到语义问题还是浏览器环境问题？
- 测试数量减少是否来自去重，而不是删除唯一边界？

### 6.4 Stateful SQLite 测试为什么存在

Stateful SQLite 测试不是普通单测的重复版本。它们用于验证内存 mock 或纯 current-schema fixture 无法证明的边界：

- 数据在真实文件和连接池中可见，并且重启或重新打开后仍保持预期状态。
- migration、schema 准备、默认数据和回退路径不会破坏已有状态。
- SQLite 写锁、连接池、项目内 coordinator 和后台 materializer 在并发下遵守优先级。
- retention、backfill、archive 和文件 I/O 不会因为争用而丢失终态记录。
- proxy runtime 的 attempt/terminal 生命周期在真实 deadline 和压力下保持完整。
- 多进程、真实路径、权限、损坏文件、gzip/archive 和恢复流程具有生产意义。

因此，优化 Stateful SQLite 的目标是减少重复 setup 和无意义的编译，而不是删除这些行为的证据。某些测试可以用 prepared schema/template 加快初始化；涉及 migration、文件、锁、跨进程或正式 deadline 的测试仍必须保留真实路径。

### 6.5 SQLite fixture 模式

| fixture 模式                     | 适合验证                                   | 优点                   | 禁止替代                        |
| -------------------------------- | ------------------------------------------ | ---------------------- | ------------------------------- |
| current-schema template + backup | 已准备 schema 下的业务读写、查询和状态转换 | 一次准备，多次隔离复制 | migration、真实路径和跨进程恢复 |
| in-memory/private DB             | 纯查询、序列化和无文件语义的逻辑           | setup 快、隔离清晰     | durability、锁、权限和 archive  |
| 每测试真实临时文件               | 文件路径、重开、恢复和权限                 | 贴近生产边界           | 不应被 template 偷换            |
| contention fixture               | lock、coordinator、retention/backfill 压力 | 证明资源争用语义       | 不能用无争用 mock               |
| archive/file I/O fixture         | gzip、损坏、恢复、文件交接                 | 证明文件边界           | 不能只测试数据库行              |

template 的正确性检查至少包括：schema/default-data parity、连接池可见性、读写双向性、每测试隔离、临时文件清理、migration 仍有真实回归和失败时可诊断。

### 6.6 后端 profile 与分片

backend test profile 应按资源和行为边界组织，而不只是按目录切割：

- lightweight：快速、低资源的逻辑回归。
- stateful-sqlite：连接池、真实 SQLite 状态、并发和持久化边界。
- archive-file-io：文件、archive、压缩、损坏和真实 I/O。
- representative scale：少量但能代表压力、deadline 和跨模块合同的确定性验收。

Stateful shard 的线程数必须由完整 profile 的重复实验决定。线程增加可能缩短纯 CPU 阶段，却增加 SQLite lock、runner oversubscription 和失败概率。分片 aggregate gate 必须能区分“某片失败”“某片没有启动”“某片被取消”和“全部通过”。

### 6.7 测试选择的禁止事项

以下做法不能作为性能优化：

- 将唯一的真实浏览器边界改成 jsdom，只因为浏览器较慢。
- 将 migration、file I/O、lock 或跨进程测试改成 template，只因为 setup 较慢。
- 用更窄的 test filter 取代原 profile，却把结果命名为完整 profile。
- 提高 retry 次数来掩盖 flaky，并把重试后的通过当作性能收益。
- 改低 timeout、阈值或断言严格度来缩短执行时间。
- 把失败测试移出 required gate，或用 continue-on-error 让 gate 通过。

## 7. 性能实验与验收方法

### 7.1 实验单位

一个实验单位是“一个明确的候选设计 + 一份完整 workload + 一组可比较的运行证据”，而不是一个命令或一个 step。候选可以是：

- workflow DAG 依赖变化；
- producer/consumer 交接；
- cache key 或缓存目录变化；
- backend profile 线程或分片变化；
- fixture template/backup；
- Playwright 到 Vitest 的测试层级迁移；
- path/run class/concurrency 变化。

一次实验只改变一个主要变量。若必须同时改变多个变量，应把它们作为一个明确的拓扑候选，并记录无法单独归因的限制。

### 7.2 Baseline 与 Candidate 矩阵

至少覆盖以下运行条件：

| 条件                     | 基线                            | 候选              | 目的                      |
| ------------------------ | ------------------------------- | ----------------- | ------------------------- |
| cold cache               | 首次或明确 miss                 | 同等 miss         | 验证首次 PR/新 runner     |
| warm cache               | 既有正确 key 命中               | 同等命中          | 验证日常重复开发          |
| partial/mismatched cache | 受控 miss 或 fallback           | 同等条件          | 验证 cache key 和恢复路径 |
| full workload            | 全部适用 required/test profiles | 完整同等 workload | 防止局部 suite 误导       |
| failure path             | producer/consumer 受控失败      | 同样失败          | 验证 fail-closed 和诊断   |
| trigger matrix           | 代表性变更域和事件              | 同样变更域和事件  | 验证按需触发不漏门禁      |

Hosted runner 存在自然抖动。对重要候选应进行多次重复，至少报告中位数、离散范围、失败/重试次数和异常原因；不要只选择最快的一次。重复次数应与实验成本相称，但不能因为实验昂贵就省略完整 workload。

### 7.3 实验执行步骤

1. 固定 base/head SHA 和 run class。
2. 记录 runner、工具链、缓存状态、并发参数和环境输入。
3. 运行基线，确认 required checks 和完整 workload 实际完成。
4. 只改变候选变量，运行同样的 candidate workload。
5. 收集 run、job、step、cache、artifact、gate 和正确性数据。
6. 计算 wall-clock、critical path、runner total 和 transfer overhead。
7. 检查测试意图、覆盖边界、失败传播和资源消耗。
8. 做出保留、继续实验或回退决定，并保存候选身份。

手动 A/B workflow 可以用于性能诊断，但必须绑定明确 SHA，不能替代 PR/merge queue/Main 的 required workflow，也不能因为 A/B 通过就自动改变生产门禁。

### 7.4 验收条件

候选只有同时满足以下条件才可保留：

| 维度   | 验收条件                                                               |
| ------ | ---------------------------------------------------------------------- |
| 墙钟   | required wall-clock 或 critical path 有可重复的改善                    |
| 总工作 | runner total 不增加，且没有把工作转移到未测量 run class                |
| 质量   | 所有适用 required checks 通过，失败传播和 aggregate gate 等价          |
| 测试   | 原测试意图、关键分支、真实边界和覆盖职责仍存在                         |
| 稳定性 | 没有新增 flaky、超时、锁竞争或重试依赖                                 |
| 传输   | cache/artifact/log 字节、restore/save/upload/download 次数无无必要增加 |
| 保存   | artifact 和诊断 retention 保持最小且仍能排障                           |
| 可追溯 | 结果绑定 SHA、run、attempt、workflow 和候选描述                        |

推荐的相对变化表达为：

```
wall_delta = candidate_required_wall - baseline_required_wall
runner_delta = candidate_runner_total - baseline_runner_total
transfer_delta = candidate_transfer - baseline_transfer
```

本 Solution 的硬规则是 runner_delta 小于或等于 0，且 transfer_delta 不得通过扩大附件或 retention 取得。wall_delta 只有在正确性和资源边界同时通过时才有意义。

### 7.5 实验记录模板

每次实验可以用以下字段保存到 PR 或实验记录中：

```text
candidate:
base_sha:
head_sha:
run_class:
workflow_runs:
runner:
toolchain:
cache_state:
workload:
changed_variable:
baseline_required_wall:
candidate_required_wall:
baseline_critical_path:
candidate_critical_path:
baseline_runner_total:
candidate_runner_total:
artifact_cache_transfer:
retention:
correctness_and_gate:
failure_retry_observation:
decision:
rollback_point:
```

这个模板的作用是让后来的人能重跑和否定结论，而不是建立一个只追加数字的时间流水账。

### 7.6 失败、重试与回退

测试失败、基础设施失败和瞬态 runner 压力必须分开：

- 代码或测试失败：回到实现诊断，不以重试掩盖。
- 可证实的瞬态基础设施失败：按仓库流程最多进行受控重试，并保留原始失败证据。
- 重试后仍失败：候选不通过，进入修复或回退。
- 只有重试通过、没有解释失败原因：不能宣称稳定性能改善。
- 新 head 会使旧 SHA 的 CI 证据失效，必须重新绑定候选。

回退应恢复到最后一个已验证的 DAG、cache namespace、fixture 模式、测试层级和 gate 合同。不要通过继续增加 retry 或延长 timeout 来保持一个无法解释的候选。

## 8. 质量边界与防回退机制

### 8.1 必须保持的行为合同

| 边界     | 必须保持                                                                    | 常见错误                                      |
| -------- | --------------------------------------------------------------------------- | --------------------------------------------- |
| 测试意图 | 原来验证的业务、错误、并发、文件或浏览器行为仍有证据                        | 只减少测试文件或断言数量                      |
| 覆盖边界 | 关键分支、真实 runtime、layout、migration、lock 和 archive 不被廉价层级替代 | 用 jsdom/template 掩盖真实边界                |
| 质量门禁 | required check、aggregate gate、merge queue 和主干合同保持 fail-closed      | skipped、continue-on-error 或窄 path 伪装成功 |
| 生产行为 | retry/backoff、deadline、SQLite 文件语义、发布路径不为测试速度改变          | 改生产参数制造更短测试                        |
| 证据身份 | 结果绑定当前 SHA、base、run class 和 workflow                               | 复用旧 commit 或不匹配的 A/B                  |
| 资源边界 | Actions 消耗不增加，附件最小，retention 最短                                | 更多 runner、更大 artifact、更长保存          |

### 8.2 Quality gate 设计

一个可复用的 gate 应具有：

1. 明确的输入 job 集合。
2. 每个输入的适用性和预期结果。
3. 对 failed、cancelled、missing、skipped 和 success 的不同处理。
4. producer 和 consumer 身份校验。
5. 对 merge queue、PR candidate 和 main 的一致质量含义。
6. 可被 branch protection 读取的稳定检查名称。

Gate 的“绿色”必须代表全部适用质量边界已证明，而不是“没有任何 job 明确报告失败”。这也是为什么按需触发需要 no-op/aggregate 语义，而不能只依赖 path filter。

### 8.3 防止性能回退

每次 workflow、测试层级、fixture 或触发器改动都应检查：

- required wall-clock 是否连续恶化；
- runner total 是否因重复准备或并行复制增加；
- cache key 是否失效或污染；
- artifact 是否变大、上传次数是否增加；
- Playwright worker、Vitest worker、Rust test thread 是否发生 oversubscription；
- flaky、timeout、retry 和 skipped 是否上升；
- required check 是否缺失、pending 或被重命名；
- 手动诊断是否意外进入默认 PR 路径。

没有统一的仓库级秒数阈值时，应使用同一 run class 的滚动 baseline 和候选对照；一旦确认 Actions 消耗增加，候选默认回退，除非主人明确批准新的资源合同。性能收益不能自行推翻消耗硬约束。

### 8.4 附件与诊断回退

失败诊断应遵循“最小充分”：

- 首先保留失败 step 的日志摘要和能定位问题的结构化结果。
- 只有浏览器失败才上传 trace、screenshot 或 video，并限制数量和 retention。
- 只有 artifact consumer 需要时才上传 binary、archive 或 bundle。
- 不上传依赖目录、完整工作区、重复 cache 或无消费者的中间目录。
- 成功路径尽量不上传大诊断文件。

如果为了排障临时扩大附件，实验结束后必须恢复最小集合；临时扩大不能成为默认 workflow 合同。

### 8.5 回退触发器

出现以下任一情况，应停止推广并回退或重新设计：

- runner total 增加；
- required gate 依赖 skipped 或 continue-on-error；
- 真实测试边界缺失或覆盖职责无法映射；
- flaky、timeout、lock contention 或 retry 率增加；
- cache/artifact/retention 无明确理由扩大；
- 新旧 run 无法绑定到同等 SHA、runner 或 workload；
- 触发器导致重复 run、pending check 或 merge queue 语义不明。

## 9. 落地与持续治理

### 9.1 分阶段落地

| 阶段     | 工作                                                             | 进入条件                 | 完成条件                                 |
| -------- | ---------------------------------------------------------------- | ------------------------ | ---------------------------------------- |
| 基线     | 画 DAG、记录 run/job/step、runner total、artifact/cache 和正确性 | 当前 required 路径可观察 | 有可比较 baseline                        |
| 重复工作 | 识别重复安装、编译、fixture 和 artifact                          | 已知重复边界             | producer/consumer 合同可验证             |
| DAG      | 去除错误串行、增加真实独立并行和 aggregate gate                  | 失败传播已明确           | critical path 改善且 runner total 不增加 |
| 测试分层 | 将确定性语义下沉到 Vitest/jsdom，保留真实边界                    | 测试意图映射完成         | 等价性和完整 workload 通过               |
| 触发器   | 按 run class、变更域和 concurrency 收敛                          | required check 规则已知  | 代表性变更矩阵通过                       |
| 治理     | 建立回退信号和复测流程                                           | 候选拓扑稳定             | 新变更能复用同一证据口径                 |

不要同时修改 DAG、测试层级、缓存、线程和触发器后再试图归因。若必须同时落地，应先拆成可独立验证的候选，或明确记录组合变化无法单独归因。

### 9.2 变更责任

| 变更类型                | 需要共同检查的对象                                              |
| ----------------------- | --------------------------------------------------------------- |
| workflow DAG/trigger    | workflow、required checks、quality-gates contract、run matrix   |
| backend profile/fixture | test runner、resource profile Spec、Stateful/Archive boundary   |
| frontend test layer     | Vitest、Playwright、route/runtime coverage、browser diagnostics |
| cache/artifact          | key、目录、consumer、retention、failure cleanup                 |
| concurrency             | PR、merge queue、main、manual diagnostic 的取消语义             |
| performance experiment  | baseline/candidate SHA、runner、cache state、完整 workload      |

Solution 负责沉淀跨任务规则；具体行为合同继续归 workflow、测试、Spec 或 ADR 所属文档管理。本 Solution 不应复制那些文档的全部细节。

### 9.3 PR 验收清单

提交性能改动时，PR 描述至少应包含：

- 改动的 workflow/test/fixture 边界。
- 基线和候选的 SHA、run class、runner 和缓存状态。
- required wall-clock、critical path、runner total 的对照。
- job 数、run 数、cache/artifact 字节和 retention 的对照。
- 测试意图和真实边界映射。
- producer/consumer/gate 的失败传播证据。
- 触发器矩阵和 merge queue 行为。
- 失败、重试和回退结论。

没有这些信息时，reviewer 只能看到代码形状，无法判断速度是否来自删测、漏门禁或增加消耗。

### 9.4 持续治理

当以下内容发生变化时，应重新检查本 Solution 的适用性：

- 新增或删除 workflow/job/required check。
- 改变 runner、工具链、缓存目录或 artifact retention。
- 新增真实浏览器、Stateful SQLite、Archive/File I/O 或性能 suite。
- 修改 path filter、merge queue、concurrency 或 gate。
- 发现重复编译、fixture setup、浏览器启动、失败重试或附件增长。

治理动作不是定期堆砌历史数字，而是重新回答“当前最慢的关键路径是什么、重复工作在哪里、消耗是否增加、质量边界是否仍然可见”。历史案例只在仍能解释当前设计时保留。

### 9.5 不应宣称的结论

以下说法需要避免：

- “某个 Vitest suite 更快，所以 CI 已优化。”
- “增加分片后单 job 更快，所以总耗时降低。”
- “失败后重跑通过，所以测试稳定。”
- “artifact 更大但保留更久，方便以后排查，所以消耗不变。”
- “文档变更没有跑测试，所以质量门禁完整。”

正确结论应绑定范围，例如“在同一 SHA、同一 runner、同一完整 workload 的三次对照中，PR required critical path 缩短，runner total 未增加，所有 required gate 与测试意图保持通过”。

## 10. 当前与过往实现映射

### 10.1 当前 workflow 拓扑

| 当前表面                                           | 仓库事实                                                                                                                               | 对本 Solution 的对应关系                                         |
| -------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| .github/workflows/ci-pr.yml                        | 由 pull_request 和 merge_group 触发；PR concurrency 可取消旧 candidate                                                                 | 对应 PR candidate/merge queue run class，取消语义必须按事件解释  |
| PR 基础质量                                        | lint、repository-tooling-checks、frontend-tests、storybook-accessibility-tests、docs-demo-build                                        | 这些 job 没有 backend archive 依赖时应保持真实并行，避免错误串行 |
| PR 浏览器路径                                      | records-overlay-e2e-producer 生成输入，records-overlay-e2e 消费并由 gate 汇总                                                          | 对应 producer/consumer/gate；producer 失败不能被 skipped 隐藏    |
| PR backend 路径                                    | backend-test-archive 后接 lightweight、Stateful SQLite 两个 shard、representative scale、Archive/File I/O；Stateful aggregate 汇总分片 | 对应不可变测试输入、资源 profile、分片和 fail-closed aggregate   |
| PR 构建路径                                        | build-pr-smoke-artifacts 产生 smoke 输入，build 和 retention-migration-validation 使用其结果                                           | 对应构建产物交接和下游验证；需要比较上传/下载与重建的完整成本    |
| .github/workflows/ci-main.yml                      | push 到 main，main concurrency 不取消；包含 candidate metadata、前端、backend archive/profile、Stateful aggregate 和 release snapshot  | 对应主干完整验证，不能被 PR candidate cancellation 破坏          |
| .github/workflows/ci-observability-performance.yml | workflow_dispatch 入口，按候选 ref 解析、producer/image/performance/result/report 分层，concurrency 不取消                             | 对应手动诊断和 A/B；不应默认变成每个 PR 的 required 工作         |

这张表描述当前拓扑事实，不等于建议立即修改这些文件。任何修改前都要重新读取当前版本、required ruleset 和 quality-gates contract。

### 10.2 当前仓库已经具备的设计

当前实现已经包含若干可复用模式：

- backend archive producer 将编译输入和多个 consumer 分离，减少重复准备。
- Stateful SQLite 使用两个 hash shard 和 aggregate check，说明分片必须有汇总合同。
- PR、main 和 manual performance 使用不同 concurrency 语义，说明取消策略属于 run class，而不是全局默认。
- frontend 测试同时存在 Vitest 和 Playwright，说明测试层级可以分工，而不是二选一。
- quality-gates contract 和 live rules check 使 workflow 结构变化有自举验证入口。

这些能力仍需按本 Solution 的指标重新测量。已有 producer、shard 或 cache 不自动证明当前关键路径最优。

### 10.3 现有 Solution、Spec 与脚本的职责

| 事实来源                                       | 负责的问题                                           | 本 Solution 如何引用                            |
| ---------------------------------------------- | ---------------------------------------------------- | ----------------------------------------------- |
| backend test runtime feedback loop             | backend compile、profile、线程、archive 和运行时反馈 | 提供资源/运行时测量案例，不复制全部后端细节     |
| main branch protection and quality gates       | required checks、gate、失败传播和主干质量合同        | 提供 fail-closed 边界，不在这里重新定义分支保护 |
| backend test resource profiles Spec            | profile、线程、分片和资源验证边界                    | 提供 backend profile 的行为来源                 |
| .github/scripts/run-backend-tests.sh           | 实际 profile、filter、cache、archive 和线程入口      | 实验必须以脚本真实 workload 为准                |
| .github/scripts/test-quality-gates-contract.sh | workflow 合同和质量门禁的机械检查                    | topology 变更后作为回归证据                     |

当前文档只抽取跨 workflow、触发器、测试层级、实验和资源边界的共性。需要修改具体合同时，应回到上述 canonical source。

### 10.4 历史 PR 的经验映射

| 历史 PR | 经验                      | 可迁移规则                                            | 不可直接推导的结论                          |
| ------- | ------------------------- | ----------------------------------------------------- | ------------------------------------------- |
| PR 1177 | 减少重复 archive setup    | 共享不可变输入，消费者只执行自身 profile              | 不能推导所有 archive 都应合并               |
| PR 1178 | 下沉重复浏览器语义覆盖    | 按测试意图把语义放到 Vitest/jsdom，保留真实浏览器边界 | 不能推导 Playwright 可以整体删除            |
| PR 1182 | 收敛 frontend validation  | 测试层级与 workflow 组合一起验收                      | 不能只比较一个前端命令                      |
| PR 1192 | 优化 SQLite fixture setup | template/backup 只替代重复 current-schema setup       | 不能替代 migration、文件、锁和 archive 回归 |

这些案例存在的价值，是为“为什么选择 producer/consumer、为什么保留真实浏览器、为什么 fixture 需要分层”提供历史证据。它们不应作为当前 CI 的 benchmark、SLA 或自动化决策输入。

### 10.5 当前仓库的推荐调查顺序

在真正改 workflow 或测试内容前，建议按以下顺序建立当前证据：

1. 从 ci-pr.yml、ci-main.yml 和 observability workflow 生成 job DAG 和 run class 矩阵。
2. 以完整 required run 测量关键路径、runner total、cache/artifact transfer 和失败传播。
3. 对 frontend 测试逐个建立“语义/DOM/layout/runtime”映射，优先识别重复 Playwright 断言。
4. 对 Stateful SQLite 记录 template setup、真实文件、lock contention、线程和分片的时间及失败率。
5. 只选择一个候选变量做 A/B，完成冷/热、成功/失败和完整 workload 验收。
6. 只有当 wall-clock 改善且 Actions 消耗不增加时，才进入 workflow 或测试实现阶段。

因此，本 Solution 的结论不是“现在就减少某些测试”，而是提供一条可审计的决策路径：先证据、再分层、再复用、再并行、再按需触发，最后用完整质量和资源指标验收。

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
