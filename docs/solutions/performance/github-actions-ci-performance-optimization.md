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

CI 变慢通常不是一个测试函数变慢，而是触发器、任务依赖、runner 准备、编译、fixture、浏览器、产物传递和门禁共同形成了较长路径。一次 job 变快，也可能只是把工作复制到了更多 runner，不能直接说明 CI 变快。

本 Solution 解决四个问题：

1. 如何找到真正决定 required 结果的关键路径。
2. 如何消除重复准备、重复编译、重复 fixture 和重复浏览器工作。
3. 如何按测试意图选择 Vitest/jsdom、Playwright、prepared SQLite 或真实文件环境。
4. 如何证明墙钟减少时 runner 总工作、附件体积和保留期没有增加。

### 1.1 目标

| 目标           | 必须观察的结果                                       |
| -------------- | ---------------------------------------------------- |
| 降低墙钟时间   | required wall-clock 或 critical path 可重复缩短      |
| 降低总执行时间 | 同一完整 workload 的 runner total 不增加             |
| 保持质量       | 测试意图、覆盖边界、质量门禁和生产行为不变           |
| 保持可解释     | 每个收益都能归因到 DAG、复用、层级、触发器或资源调整 |

### 1.2 硬约束与非目标

- GitHub Actions 消耗不增加是硬约束，不把降低账单写成独立目标。
- 不删除测试意图，不放宽断言、阈值、deadline、质量门禁或失败传播。
- 不修改生产行为来制造更短的测试。
- 不通过更大 runner、无上限并行、重复 run、大附件或过长 retention 换取墙钟收益。
- 不把一次局部测试、一次缓存命中或一次重试结果写成 whole-CI 结论。

本 Solution 是设计和验收规则，不是对所有仓库规定相同的 job 名称、runner、分片数或测试工具。

### 1.3 优化结果的最低交付物

每个候选方案都应能回答：

- 基线和候选绑定到什么 SHA、事件、runner 和缓存状态？
- 哪段关键路径被缩短，哪段工作被复用？
- 哪些测试意图由哪个层级继续证明？
- runner total、run/job 数、cache/artifact 字节和 retention 是否增加？
- 失败如何传播，候选如何回退？

无法回答这些问题时，只能称为诊断结果，不能称为完成优化。

## 2. CI 性能指标与成本不增加约束

### 2.1 统一指标

| 指标                | 定义                                                  | 用途                     |
| ------------------- | ----------------------------------------------------- | ------------------------ |
| required wall-clock | 第一条适用 required job 启动到最终 required gate 完成 | 开发者等待时间           |
| critical path       | 按实际依赖从触发到 gate 的最长链                      | 判断并行是否有效         |
| job wall time       | 单个 job 的启动到结束                                 | 找到 top offender        |
| runner total        | 所有适用 job wall time 之和                           | 防止用更多 runner 换墙钟 |
| setup time          | checkout、工具链、依赖、cache 和 fixture              | 判断准备是否重复         |
| test time           | 实际测试或浏览器命令                                  | 区分测试慢和准备慢       |
| transfer time       | cache、artifact、解压和聚合                           | 判断复用是否被传输抵消   |
| correctness         | pass、fail、cancelled、skipped、retry、gate           | 防止速度来自隐藏失败     |

统一计算口径：

```text
required_wall_clock = required_gate_end - first_applicable_required_start
critical_path = longest effective dependency chain
runner_total = sum(applicable job wall time)
```

这些公式用于同 workload 比较，不是 GitHub 账单计算器。账单判断还受 runner 类型、计费粒度和组织配置影响，本 Solution 不预设价格。

### 2.2 基线与候选的最小记录

| 类别 | 必须记录                                             |
| ---- | ---------------------------------------------------- |
| 身份 | base SHA、head SHA、run class、workflow、run/attempt |
| 环境 | runner、工具链、依赖版本、浏览器版本                 |
| 缓存 | cold、warm、miss、partial hit                        |
| 时间 | job 和关键 step 的 startedAt、completedAt            |
| 资源 | 线程、worker、连接池、分片和共享 server              |
| 传输 | cache/artifact 的次数、目录、上传/下载字节           |
| 结果 | required checks、flaky、retry、skipped、gate         |

基线必须包含完整适用 workload。单个 Vitest、Playwright、backend profile 或 producer step 只能说明局部现象。

### 2.3 Actions 消耗不增加的硬门禁

| 检查         | 通过条件                             | 不通过时                   |
| ------------ | ------------------------------------ | -------------------------- |
| run/job 数   | 没有重复触发和无功能价值的新 job     | 回退触发器或拓扑           |
| runner total | 同等 workload 不增加                 | 拒绝候选，即使墙钟变短     |
| cache        | key、目录、次数和字节无无必要增加    | 收窄 key 或目录            |
| artifact     | 只上传 consumer 必需的最小文件       | 移除工作区、依赖和重复报告 |
| retention    | 仅覆盖交接、诊断和审计所需的最短期限 | 缩短 retention             |
| correctness  | gate、覆盖边界和失败传播等价         | 回退或重新设计             |

“不增加”必须在同等 workload 上比较。如果只是减少 PR 工作、把同样工作转移到 Main 或手动 run，必须明确披露完整生命周期，不能宣称整体消耗不增加。

### 2.4 Cache、Artifact 与诊断边界

| 对象     | 应保存                                              | 不应保存                                  |
| -------- | --------------------------------------------------- | ----------------------------------------- |
| cache    | 可由 source key 重建的依赖和编译输入                | 测试结果、失败诊断、完整工作区            |
| artifact | 下游必须消费的不可变 binary、archive、bundle 或报告 | node_modules、完整 target、浏览器安装目录 |
| 失败诊断 | 定位失败所需的日志摘要、trace、截图或 video         | 每次成功 run 的完整环境快照               |

上传前必须说明消费者和生命周期。失败路径才需要的诊断不应在成功路径全部上传；没有消费者的文件不应为了“以后可能有用”长期保存。

### 2.5 指标解释

- wall-clock 降低但 runner total 增加：通常是并行复制，候选不通过。
- runner total 降低但 wall-clock 不变：工作不在关键路径，不能宣称开发者等待变快。
- 测试命令变快但 job 不变：setup、下载、fixture 或 gate 仍是瓶颈。
- 平均值变快但 flaky/retry 增加：收益被稳定性损失抵消。
- 只有 warm cache 变快：必须补充 cold cache 或明确限定结论。

## 3. CI 瓶颈识别方法

### 3.1 五步定位

1. **固定入口**：确认是 PR、merge queue、main 还是手动诊断。
2. **画出 DAG**：列出 job、needs、条件、producer、consumer 和 gate。
3. **回溯关键路径**：从最终 required gate 回到最早启动的依赖。
4. **拆分时间**：checkout、工具链、cache、compile、fixture、test、browser、upload、cleanup。
5. **建立单变量假设**：选择一个依赖、缓存、分片、fixture、层级或触发器变量做实验。

先定位再修改测试。Stateful job 慢可能来自编译、schema setup、锁等待、线程过高或 gate 等待，这些问题的修复完全不同。

### 3.2 症状、证据和候选

| 症状                 | 证据                             | 典型候选                                 |
| -------------------- | -------------------------------- | ---------------------------------------- |
| job 都等待同一个 job | needs 图和启动时间               | 缩小 producer，解除错误串行              |
| compile 占大部分时间 | compile step、cache key、archive | 精确 cache key 或不可变 producer         |
| test 很短但 job 很长 | setup、下载、上传时间            | 缩小 artifact，复用准备工作              |
| 线程越多越慢         | 线程矩阵、CPU、lock 日志         | 限制 profile 线程，减少 oversubscription |
| Playwright 不稳定    | worker、server readiness、trace  | 固定 worker，拆出语义测试                |
| 文档变更也跑全套     | path、event、run 数              | 变更分类和可见 no-op gate                |
| 上游失败但 gate 绿色 | needs result、if、gate 日志      | fail-closed aggregate gate               |

历史数据只用于形成假设，不能替代当前候选的完整测量。

### 3.3 关键路径与 runner total

```text
event
  ├── lint/tooling ───────┐
  ├── frontend ───────────┤
  ├── docs/demo ──────────┼── aggregate gate
  └── backend producer ───┘
          ├── lightweight
          ├── stateful shards
          └── archive/file-io
```

没有真实数据依赖的分支应并行；有共同输入的分支应共享 producer。把一条十分钟工作复制成两条五分钟工作可能缩短墙钟，但会增加 runner total；正确的优化必须同时减少依赖等待和重复工作。

### 3.4 当前仓库的诊断入口

当前仓库已有的事实来源按职责使用：

| 来源                                                     | 用来回答                                            |
| -------------------------------------------------------- | --------------------------------------------------- |
| ci-pr.yml、ci-main.yml、ci-observability-performance.yml | 触发器、DAG、concurrency、producer/consumer 和 gate |
| backend runtime Solution                                 | 编译、profile、线程、archive 和历史时序             |
| quality-gates Solution                                   | required check、失败传播和主干质量边界              |
| backend resource profiles Spec                           | profile、线程、分片和资源行为                       |
| run-backend-tests.sh                                     | 实际 filter、archive、cache 和线程入口              |
| test-quality-gates-contract.sh                           | workflow 合同的机械校验                             |

历史 run 和 PR 的具体映射集中在第 10 章，避免在诊断方法中重复堆叠数字。

## 4. Workflow 内部设计

### 4.1 DAG 目标

```text
event
  -> change classification
  -> immutable producer(s)
  -> independent consumers
  -> fail-closed aggregate gate
  -> required check
```

依赖只表达真实数据依赖。lint、frontend、docs/demo 和 backend producer 没有共同输入时可以并行；多个 backend profile 共享同一测试 archive 时由 producer 统一提供。

### 4.2 Producer、Consumer、Gate 合同

| 角色       | 责任                                        | 禁止事项                                |
| ---------- | ------------------------------------------- | --------------------------------------- |
| Classifier | 确定 run class、变更域和适用 workload       | 让 required check 无理由消失            |
| Producer   | 在固定 SHA 上生成完整、不可变、可校验的输入 | 上传半成品或让 consumer 重编译          |
| Consumer   | 下载必要输入并运行自身 profile/filter       | 重复 producer 的构建和 fixture          |
| Gate       | 汇总所有适用结果并显式失败                  | 把 skipped、missing 或 cancelled 当成功 |

Producer 输出应有文件清单、身份和消费者；只有完整成功后才能上传。Consumer 先校验输入，再执行测试；成功路径不上传大诊断。Gate 必须同时检查 producer result、每个 consumer result 和 applicability。

设计意图可以表示为：

```yaml
gate:
  needs: [producer, consumer-a, consumer-b]
  if: always()
  result: fail when any applicable result is failed, cancelled, missing, or skipped
```

always 只保证 gate 有机会观察上游，不代表 gate 可以无条件成功。

### 4.3 并行、分片和资源边界

分片前先记录分片键、测试意图、线程、连接、临时空间、文件和 aggregate 规则。Stateful SQLite、Archive/File I/O 和 Playwright 不能只按测试数量平均切片，因为文件、锁和真实 deadline 可能远比纯语义测试重。

选择线程或浏览器 worker 时，要同时测量：

- 完整 profile 的 wall-clock；
- runner total 和 setup/transfer；
- CPU oversubscription、SQLite lock 和 server 争用；
- flaky、timeout 和 retry；
- 分片 aggregate 的缺片、取消和失败语义。

只在单次运行中增加并发而不记录资源和失败率，不是有效优化。

### 4.4 Cache、Artifact 和源码复用

| 决策问题                         | Cache  | Artifact/Producer |
| -------------------------------- | ------ | ----------------- |
| 内容可由 source key 重建？       | 适合   | 通常不适合        |
| 下游需要同一 run 的精确输出？    | 不适合 | 适合              |
| 内容是当前 run 的测试结果/诊断？ | 不适合 | 适合              |
| 多个 job 需要不可变身份？        | 可选   | 必须              |

编译 cache key 应覆盖 lockfile、manifest、工具链、profile 和源码 fingerprint。先比较 restore、解压、compile、test 和 job wall time，再判断 cache 命中是否有收益。Artifact 只承载 consumer 必需的不可变交接物，retention 保持最短。

### 4.5 Concurrency 与取消

普通 PR candidate 可以按 PR 编号取消旧候选；merge queue、main、发布完成路径和完整诊断通常需要不取消或独立的 concurrency。设计时必须验证：

- 取消旧 run 不会留下 pending required check；
- merge queue 不会被普通 PR 取消规则误伤；
- manual diagnostic 按 SHA 隔离；
- cancellation 不会被用来隐藏失败或绕过 gate。

## 5. 按需触发策略

### 5.1 Run class

| run class              | 入口                              | 目的                          | 默认取消               |
| ---------------------- | --------------------------------- | ----------------------------- | ---------------------- |
| PR candidate           | pull_request                      | review 反馈和 required checks | 同一 PR 的旧候选可取消 |
| merge queue            | merge_group                       | 最终合并候选验证              | 不采用普通 PR 取消     |
| Main                   | push 到 main                      | 主干完整验证和发布输入        | 不取消                 |
| Performance/diagnostic | workflow_dispatch 或明确 schedule | 绑定 SHA 的 A/B 和资源诊断    | 不取消                 |

手动诊断不能替代 required workflow；PR 快速反馈也不能替代主干完整验证。

### 5.2 变更域矩阵

| 变更域                           | PR candidate                               | merge queue/Main       | manual diagnostic       |
| -------------------------------- | ------------------------------------------ | ---------------------- | ----------------------- |
| 仅文档                           | 文档门禁和可见 no-op 结果                  | 仓库既有保护路径       | 不自动触发              |
| 前端语义/组件                    | Vitest、类型/构建、必要浏览器边界          | 完整前端 required 边界 | 可做浏览器性能          |
| layout、viewport、导航、PWA、SSE | 保留真实 Playwright 边界                   | 完整真实浏览器边界     | 绑定 SHA 做性能诊断     |
| backend source/profile           | archive producer、适用 profiles、aggregate | 主干完整 backend       | 线程、fixture、资源 A/B |
| workflow、质量门禁、共享 fixture | 相关完整合同，不走窄路径                   | 完整保护路径           | 可做拓扑实验            |
| 纯性能调查                       | 不默认进入 PR                              | 按主干策略             | 手动完整 workload       |

这是 workload 选择规则，不是直接跳过 required check 的授权。

### 5.3 变更分类与 path filter

分类顺序：

1. workflow、质量门禁、构建输入、测试脚本或共享 fixture 改动，进入相关完整验证。
2. 真实浏览器、文件、数据库、migration、锁或跨进程改动，保留真实环境测试。
3. 确认只影响不参与运行时的文档或说明时，才减少不适用的昂贵 workload。
4. 无法证明变更域时，选择更宽的 workload。

Path filter 不能让 required check 长期缺失或 pending。可使用显式 no-op/aggregate job，但必须报告“不适用”的原因，不能把真正失败伪装成成功。每次 filter 改动至少验证文档-only、前端-only、backend-only、workflow-only、跨域和 merge queue。

### 5.4 防止重复触发

调整触发器后检查：

- 同一 SHA 是否产生多个等价 run；
- label 或 metadata 改动是否重复启动 CI；
- 旧 candidate 取消后是否留下 producer、artifact 或 pending check；
- merge queue 是否重新执行必要检查；
- manual diagnostic 是否只由明确入口启动。

目标不是让 run 数越少越好，而是让适用 workload 只执行一次且质量边界完整可见。

## 6. 测试组合与执行层级

### 6.1 测试意图矩阵

| 测试意图                                           | 首选层级                       | 必须保留真实浏览器/环境的情况   |
| -------------------------------------------------- | ------------------------------ | ------------------------------- |
| 纯函数、序列化、请求构造、状态机                   | Vitest 单测                    | 无                              |
| React 状态、事件、DOM 语义、可访问名称             | Vitest + jsdom                 | 真实 layout、focus 或浏览器 API |
| CSS geometry、overflow、resize、scroll、focus ring | Playwright                     | 必须保留真实 viewport           |
| 真实 route、导航、keyboard/touch、PWA runtime      | Playwright                     | 必须保留                        |
| SSE、网络失败、server readiness                    | Playwright/集成环境            | 必须保留真实服务边界            |
| current-schema DB 读写，不验证文件和迁移           | template/backup 或隔离 fixture | 真实文件、连接池和跨进程语义    |
| migration、权限、损坏文件、archive/gzip            | Stateful/Archive profile       | 必须保留真实文件                |
| lock、retention/backfill、正式 deadline 压力       | Stateful/Archive profile       | 必须保留真实竞争                |

浏览器只承担浏览器能证明的行为。测试数量减少只有在语义、错误、关键分支和真实边界都有等价证据时才有意义。

### 6.2 Playwright 下沉规则

对候选用例按四个问题判断：

1. 是否依赖 viewport、CSS 计算、scroll、focus、keyboard 或 touch？
2. 是否依赖真实导航、SSE、网络失败、Service Worker 或 PWA runtime？
3. 是否只是给定输入下的文本、状态、事件和错误分支？
4. jsdom 是否会提供不同或不完整的浏览器语义？

只满足第 3 项时，优先下沉到 Vitest/jsdom；满足第 1 或第 2 项时保留 Playwright；第 4 项不确定时先保留浏览器证据。

迁移需要保留一张映射：

| 原浏览器证据              | 新 Vitest 证据                                | 保留的 Playwright 证据              |
| ------------------------- | --------------------------------------------- | ----------------------------------- |
| 文本、状态和错误分支      | 相同输入下的组件/handler 结果                 | 至少一个真实 route/render smoke     |
| route 数据状态            | mock network 下的 loading/empty/error/success | 真实 route 和网络失败               |
| formatter 和 tooltip 语义 | formatter/model 单测                          | 真实 geometry、resize 或 canvas/SVG |
| mobile overflow           | 不完全下沉                                    | 真实 viewport、scroll、touch        |

### 6.3 Stateful SQLite 测试为什么存在

Stateful SQLite 不是普通单测的重复版本。它验证：

- 真实文件、连接池、重开和持久化可见性；
- migration、schema/default data 和恢复路径；
- SQLite 写锁、coordinator、后台 materializer 和并发优先级；
- retention、backfill、archive、文件 I/O 和终态记录；
- proxy runtime 的 attempt/terminal 生命周期和正式 deadline；
- 多进程、权限、损坏文件、gzip/archive 和恢复。

因此优化目标是减少重复 setup 和无意义编译，不是把这些行为改成 mock。

### 6.4 Fixture 模式

| 模式                             | 适合验证                              | 不能替代                        |
| -------------------------------- | ------------------------------------- | ------------------------------- |
| current-schema template + backup | 已准备 schema 的隔离读写和状态转换    | migration、真实路径、跨进程恢复 |
| in-memory/private DB             | 无文件语义的查询和逻辑                | durability、锁、权限、archive   |
| 每测试真实临时文件               | 文件路径、重开、恢复、权限            | 不应被 template 偷换            |
| contention fixture               | lock、coordinator、retention/backfill | 不能用无争用 mock               |
| archive/file I/O fixture         | gzip、损坏、恢复和文件交接            | 不能只测数据库行                |

Template 落地前检查 schema/default-data parity、连接池可见性、读写隔离、清理和真实回归仍存在。

### 6.5 后端 profile 与禁止事项

profile 应按资源和行为组织：

- lightweight：低资源逻辑回归；
- stateful-sqlite：真实 SQLite 状态、连接池和并发；
- archive-file-io：文件、压缩、损坏和 I/O；
- representative scale：代表压力、deadline 和跨模块合同的确定性验收。

不能通过以下方式加速：

- 把唯一真实浏览器边界改成 jsdom；
- 把 migration、file I/O、lock 或跨进程测试改成 template；
- 用更窄 filter 代替完整 profile；
- 增加 retry 掩盖 flaky；
- 降低 timeout、阈值或断言严格度；
- 把失败测试移出 required gate。

## 7. 性能实验与验收方法

### 7.1 实验控制面

每次实验固定：

- 同一 base/head SHA、run class、runner 和工具链；
- 同一完整 profile、浏览器 suite、artifact/gate 流程；
- 同等 cold、warm 或 cache miss 条件；
- 同样的失败重试规则和统计窗口；
- 一个主要候选变量。

候选变量可以是 DAG 依赖、producer/consumer、cache key、线程/分片、fixture、Playwright/Vitest 层级或触发器。多个变量必须明确写成组合候选，不能假装可单独归因。

### 7.2 实验矩阵

| 条件                      | 目的                   |
| ------------------------- | ---------------------- |
| cold cache                | 首次 PR 和新 runner    |
| warm cache                | 日常重复开发           |
| partial/miss cache        | key 和 fallback 正确性 |
| full workload             | 防止局部 suite 误导    |
| producer/consumer failure | fail-closed 和诊断     |
| trigger matrix            | path、事件和取消语义   |

重要候选应多次运行并报告中位数、离散范围、失败/重试次数和异常原因。不能只挑最快的一次。

### 7.3 执行与记录

1. 记录 baseline。
2. 只改变候选变量。
3. 运行完整 candidate workload。
4. 收集 run、job、step、cache、artifact、gate 和正确性。
5. 比较 wall-clock、critical path、runner total、transfer 和失败率。
6. 决定保留、继续实验或回退。

建议记录：

```text
candidate:
base_sha:
head_sha:
run_class:
runner:
toolchain:
cache_state:
workload:
changed_variable:
baseline_required_wall:
candidate_required_wall:
baseline_runner_total:
candidate_runner_total:
transfer_and_retention:
correctness_and_gate:
retry_observation:
decision:
rollback_point:
```

### 7.4 验收门槛

| 维度   | 必须满足                                               |
| ------ | ------------------------------------------------------ |
| 墙钟   | required wall-clock 或 critical path 可重复改善        |
| 总工作 | runner total 不增加，且没有把工作转移到未测量入口      |
| 质量   | required checks、gate、失败传播和测试意图等价          |
| 稳定性 | 没有新增 flaky、超时、锁竞争或 retry 依赖              |
| 传输   | cache/artifact/log 次数、字节和 retention 无无必要增加 |
| 追溯   | 结果绑定 SHA、run、attempt、workflow 和候选描述        |

可用以下差值表达结果：

```text
wall_delta = candidate_required_wall - baseline_required_wall
runner_delta = candidate_runner_total - baseline_runner_total
transfer_delta = candidate_transfer - baseline_transfer
```

硬规则是 runner_delta 小于或等于 0；不能通过扩大附件或 retention 取得 transfer_delta 的表面收益。

### 7.5 失败、重试与回退

- 代码/测试失败回到实现诊断，不用重试掩盖。
- 只有有证据的瞬态基础设施失败才允许受控重试。
- 重试后仍失败，候选不通过。
- 只有重试通过而没有解释原失败，不能宣称稳定收益。
- 新 head 会使旧 SHA 证据失效，必须重新绑定。

回退恢复最后一个已验证的 DAG、cache namespace、fixture、测试层级和 gate 合同；不通过继续增加 retry 或延长 timeout 保持“绿色”。

## 8. 质量边界与防回退机制

### 8.1 不可改变的合同

| 合同     | 保持什么                                                       | 不能做什么                          |
| -------- | -------------------------------------------------------------- | ----------------------------------- |
| 测试意图 | 业务、错误、并发、文件和浏览器行为仍有证据                     | 只减少断言或文件                    |
| 真实边界 | layout、runtime、migration、lock、archive 和 deadline 仍被证明 | 用 jsdom/template 偷换              |
| 质量门禁 | required、aggregate、merge queue、main 保持 fail-closed        | skipped、continue-on-error 隐藏失败 |
| 生产行为 | retry/backoff、deadline、SQLite 文件语义和发布行为不变         | 改生产参数制造速度                  |
| 资源     | Actions 消耗不增加，附件最小，retention 最短                   | 更多 runner、大附件、长保存         |

第 2 章是资源和附件的权威规则，第 6 章是测试层级和真实边界的权威规则，第 7 章是实验验收的权威规则；本章只定义不得违反的回退条件。

### 8.2 Gate 的最低要求

Gate 必须有明确输入 job、适用性、成功/失败/取消/missing/skipped 处理、producer 身份校验和稳定的 required check 名称。上游失败必须成为显式 failed gate，不能因为下游自然 skipped 而被误判成功。

### 8.3 回退信号

出现以下任一情况，停止推广：

- runner total 增加；
- required gate 依赖 skipped 或 continue-on-error；
- 真实测试边界无法映射；
- flaky、timeout、lock contention 或 retry 增加；
- cache/artifact/retention 无理由扩大；
- 新旧 run 无法绑定同等 SHA、runner 或 workload；
- 触发器产生重复 run、pending check 或 merge queue 语义不明。

## 9. 落地与持续治理

### 9.1 实施顺序

| 阶段     | 工作                                   | 通过条件                                 |
| -------- | -------------------------------------- | ---------------------------------------- |
| 基线     | DAG、时间、runner total、传输和正确性  | 有完整可比较 baseline                    |
| 去重     | 安装、编译、fixture、artifact          | producer/consumer 合同可验证             |
| DAG      | 独立并行、profile 分片、aggregate gate | critical path 改善且 runner total 不增加 |
| 测试分层 | 语义下沉，真实边界保留                 | 等价性和完整 workload 通过               |
| 触发器   | run class、path、concurrency           | 代表性变更矩阵通过                       |
| 治理     | 回退信号、复测和引用                   | 后续改动可复用同一口径                   |

不要同时无记录地改变 DAG、缓存、线程、fixture 和测试层级；否则无法知道收益或回归来自哪里。

### 9.2 PR 证据清单

性能改动的 PR 描述至少应包含：

- workflow/test/fixture 变更边界；
- base/head SHA、run class、runner、缓存状态；
- required wall-clock、critical path、runner total 对照；
- run/job 数、cache/artifact 字节和 retention 对照；
- 测试意图与真实边界映射；
- producer/consumer/gate 失败传播；
- 触发器矩阵、merge queue 行为、失败和回退结论。

没有这组证据时，不能判断速度来自真实优化、删测、漏门禁还是增加消耗。

### 9.3 持续治理

新增或修改 workflow/job/required check、runner、工具链、cache、artifact、真实浏览器、Stateful SQLite、path filter、concurrency 或 gate 时，重新检查本 Solution 的适用性。

治理只回答四个问题：当前关键路径是什么、重复工作在哪里、消耗是否增加、质量边界是否仍然可见。历史数字只有在能解释当前决策时才保留。

## 10. 当前与过往实现映射

### 10.1 当前 workflow

| 表面                      | 当前事实                                                                                                     | 对应策略                                             |
| ------------------------- | ------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------- |
| CI PR                     | pull_request 和 merge_group；PR candidate 可取消旧 run                                                       | PR/merge queue 分开解释 concurrency 和 required 语义 |
| PR 基础质量               | lint、repository tooling、frontend、Storybook、docs/demo                                                     | 无共享输入时并行，避免错误串行                       |
| PR 浏览器路径             | records-overlay producer 与 consumer/gate                                                                    | producer 一次生成，consumer 只读使用，失败显式传播   |
| PR backend 路径           | archive producer、lightweight、Stateful 两 shard、representative scale、Archive/File I/O、Stateful aggregate | 共享测试输入，按资源 profile 分片并 fail-closed 汇总 |
| PR smoke/build 路径       | smoke artifact producer、build、retention migration validation                                               | 比较上传/下载与重建的完整成本                        |
| CI Main                   | push 到 main，main concurrency 不取消，含前端、backend、Stateful aggregate 和 release snapshot               | 主干完整验证不受 PR cancellation 破坏                |
| Observability Performance | workflow_dispatch，resolve、producer、image、performance、result、report 分层，concurrency 不取消            | 绑定 SHA 的手动诊断，不默认进入每个 PR               |

上述是当前拓扑映射，不是本次直接修改 workflow 的授权。

### 10.2 当前已有的可复用能力

- backend archive producer 已将编译输入与多个 consumer 分离；
- Stateful SQLite 已有两个 hash shard 和 aggregate check；
- PR、main、manual performance 使用不同 concurrency 语义；
- frontend 同时有 Vitest 和 Playwright，可按测试意图分层；
- quality-gates contract 和 live rules check 提供 workflow 自举校验。

这些能力仍需用第 2、7 章的指标重新验证，不能自动等同于最优拓扑。

### 10.3 现有 Solution、Spec 与脚本

| 来源                                     | 事实职责                                     | 本 Solution 的用法        |
| ---------------------------------------- | -------------------------------------------- | ------------------------- |
| backend test runtime feedback loop       | compile、profile、线程、archive 和运行时反馈 | 提供 backend 性能案例     |
| main branch protection and quality gates | required check、gate、失败传播               | 提供 fail-closed 边界     |
| backend resource profiles Spec           | profile、线程、分片、资源边界                | 提供 backend 行为来源     |
| run-backend-tests.sh                     | filter、archive、cache、线程入口             | 保证实验使用真实 workload |
| test-quality-gates-contract.sh           | workflow 合同机械检查                        | topology 变更后的回归证据 |

具体行为合同仍以这些 canonical source 为准，本 Solution 不复制它们的全部内容。

### 10.4 历史案例与可迁移结论

| 案例                 | 事实                                                                      | 可迁移结论                                      | 不能推出                               |
| -------------------- | ------------------------------------------------------------------------- | ----------------------------------------------- | -------------------------------------- |
| Main run 31706131099 | Stateful 617s wall，其中 compile 143s、test 404s，backend jobs 合计 1257s | 分开看 compile、profile、fixture、test 和 DAG   | 不是当前 SLA                           |
| PR run 31825458818   | Lint、backend profiles 和 Build Artifacts 有独立耗时                      | setup、产物和 profile 都可能是瓶颈              | 不能推导所有 PR 相同                   |
| PR 1177              | 减少重复 archive setup                                                    | 共享不可变输入，consumer 不重复构建             | 不是所有 archive 都应合并              |
| PR 1178              | 下沉重复前端语义覆盖                                                      | 语义进 Vitest/jsdom，真实边界留在 Playwright    | 不能整体删除 Playwright                |
| PR 1182              | 收敛 frontend validation                                                  | 测试层级与 workflow 组合一起验收                | 不能只看单个 suite                     |
| PR 1192              | 优化 SQLite fixture setup                                                 | template/backup 只替代重复 current-schema setup | 不能替代 migration、文件、锁和 archive |

这些案例存在的唯一价值，是解释当前设计选择并提供可追溯来源；它们不是当前 CI 的 benchmark 或自动决策输入。

### 10.5 推荐调查顺序

1. 从三个 workflow 生成当前 job DAG 和 run class 矩阵。
2. 用完整 required run 测量关键路径、runner total、传输和失败传播。
3. 为 frontend 用例建立语义、DOM、layout、navigation、runtime 映射。
4. 为 Stateful SQLite 记录 template setup、真实文件、锁、线程和分片结果。
5. 每次只选一个候选变量做冷/热、成功/失败和完整 workload 对照。
6. 只有 wall-clock 改善且 Actions 消耗不增加时，才实施候选。

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
