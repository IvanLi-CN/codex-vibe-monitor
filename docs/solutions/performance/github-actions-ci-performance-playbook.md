---
title: GitHub Actions CI performance and cost optimization
module: ci
problem_type: performance
component: GitHub Actions workflows and test execution
tags:
  - github-actions
  - ci
  - wall-clock
  - runner-cost
  - test-selection
  - artifacts
  - caching
  - triggers
status: active
related_specs:
  - docs/specs/q7yt7-backend-test-resource-profiles/SPEC.md
---

# GitHub Actions CI 性能与成本优化

## Context

CI 优化的目标不是单独缩短某个 Job，而是在测试可信度不下降的前提下，同时改善：

- 开发者等待的反馈墙钟时间。
- 所有 Runner 的总执行时间和实际用量。
- Artifact、Cache、日志和上传下载带来的存储与传输成本。

成本可控是性能优化的前置约束。并行化只有在墙钟收益值得新增 Runner 消耗时才成立；测试迁移只有在保留原验证意图时才成立；长期保存的产物只有在有明确调试、发布或审计用途时才成立。

## Symptoms

先把一次 CI 运行拆成同一条时间线：

```text
触发与排队 -> 环境准备 -> 构建 -> fixture 准备 -> 测试执行 -> 汇总 -> 产物保存
```

分别记录每一段的耗时、重复次数、是否可以并行、是否必须在当前事件执行，以及它对以下指标的影响：

- 关键路径和反馈墙钟时间。
- Runner 总执行时间和计费使用量。
- Artifact、Cache 和日志的体积与保留量。

不要只用一个 Workflow 的总时长定位瓶颈，也不要把排队、冷编译、fixture 初始化和测试执行混成一个“慢测试”。

## Root cause

CI 变慢通常来自以下几类问题：

1. **重复工作**：多个 Job 重复安装依赖、编译、启动服务或构造相同 fixture。
2. **验证范围过大**：所有代码变更都触发完整的高成本套件。
3. **执行层级错误**：纯逻辑使用真实浏览器，或窄状态测试启动完整业务流水线。
4. **依赖关系过度串行**：独立任务被不必要地放在同一条关键路径上。
5. **资源争用**：并发的浏览器、SQLite、磁盘或容器任务互相拖慢。
6. **结果复用不可靠**：缓存 key 过宽、Artifact 混用或 Producer 失败被隐藏。

## Resolution

### 1. 使用四种优化动作

任何优化应明确属于以下一种或多种动作：

- **减少**：删除重复验证，或只选择与变更相关的验证。
- **复用**：一次构建、多次消费；复用安全的 fixture、缓存和 Artifact。
- **并行**：把无依赖任务移出关键路径，并用实测结果决定分片。
- **延迟**：将性能、诊断和完整回归放到合适的事件或手动入口。

### 2. 将 Workflow 设计为执行计划

推荐的抽象拓扑是：

```text
事件识别
  -> 变更分类
  -> 生成验证计划
  -> Producer 生成公共产物
  -> Consumers 并行执行资源档位
  -> Gate 汇总并传递失败
```

- Producer 负责一次性构建可复用产物。
- Consumer 按测试类型、资源特征或风险等级执行。
- Gate 必须检查上游结果和自身结果，不能用 `skipped` 隐藏失败。
- Cache key 必须包含依赖、工具链、源码和执行 profile 等兼容性输入。
- Artifact 按运行实例、提交和 attempt 隔离，避免消费旧结果。
- 同一 PR 的过期 candidate 可以取消；Main、merge queue、release 和 required gate 必须使用独立的取消语义。

### 3. 按风险和成本选择触发层级

| 层级         | 默认用途                               |
| ------------ | -------------------------------------- |
| Required     | 每个相关 PR 必须通过的快速验证         |
| Change-aware | 根据变更范围追加的相关验证             |
| Full         | Main、merge queue 或定时执行的完整回归 |
| On-demand    | 性能、诊断、A/B 和高成本实验           |

路径过滤、标签和 `workflow_dispatch` 只能改变可选验证或验证计划，不能让必需检查静默消失。按需策略必须有定期完整验证，用来发现选择规则漂移和漏测。

### 4. 按测试意图选择执行层级

- 纯函数、数据转换、状态组合和请求参数：优先单元测试或 Vitest/jsdom。
- 组件语义和有限交互：优先组件测试。
- 真实布局、几何、焦点、触摸、浏览器 API 和关键端到端路径：保留真实浏览器。
- 数据库迁移、文件路径、压缩、损坏和锁竞争：保留真实文件或数据库环境。
- 只验证当前 schema 和默认数据的重复 fixture：在证明隔离和语义等价后复用模板。

降低真实浏览器或真实环境的比例，不等于删除测试；每次迁移都要说明被保留的验证意图和仍由高成本层覆盖的边界。

### 5. 控制 GitHub Actions 成本

#### Runner 与并发

- 同时比较关键路径、Runner 总时间和 GitHub 实际使用量。
- 不用无限增加 Job 数量换取墙钟下降。
- 对可替换的 PR candidate 使用 per-PR cancellation，避免旧提交继续消耗资源。
- 对不可替换的 required、mainline、发布和手动任务使用独立 namespace 与明确的排队策略。

#### Artifact、Cache 与日志

- Workflow 传递产物只保存下游真正需要的内容。
- 失败诊断的截图、视频、Trace 和原始日志按需上传。
- 成功运行默认保存摘要；完整原始报告需要明确用途。
- 不上传 `node_modules`、`target`、浏览器缓存或可重新生成的构建缓存。
- 设置每类 Artifact 的大小预算，上传前检查体积并避免原始文件与重复压缩副本同时保存。
- 每次上传显式设置最短可用的 `retention-days`；普通 PR 产物短期保存，发布或审计产物才允许更长保留。
- Cache 用于未来复用，Artifact 用于当前运行传递或诊断；两者不能混为一谈。

具体计费规则和保留上限以 GitHub 当前账单与仓库策略为准，不把某个价格或默认天数固化为项目常量。

### 6. 用完整实验验证收益

采用相同提交、Runner、工具链和缓存条件的 control/candidate 对照；冷缓存与热缓存分别统计，并交错重复执行。至少同时报告：

- 关键路径墙钟时间。
- Runner 总执行时间。
- GitHub 实际使用量或可对应的计费指标。
- Artifact、Cache 和日志体积。
- 重试、取消、超时和排队情况。

局部 Job 变快、另一组运行超时或 workload 不完整时，不能宣称整条 CI 已优化。

### 7. 分阶段推广

```text
建立基线
  -> 消除重复工作
  -> 复用构建与 fixture
  -> 调整并行和分片
  -> 调整测试执行层级
  -> 引入按需触发
  -> 持续监控成本与回归
```

每次推广都应记录目标指标、质量边界、验收条件和回滚方式。

## Guardrails / Reuse notes

- 不能通过跳过 required check、放宽阈值或缩短生产超时制造速度收益。
- Producer 失败必须成为显式失败；下游被跳过不能代表成功。
- 缓存不能跨不兼容的源码、工具链、feature 或 profile 复用。
- 真实 migration、文件 I/O、锁竞争、浏览器布局和生产时序不能全部替换成模拟逻辑。
- 浏览器 worker、数据库连接、磁盘和容器并发必须受控。
- Artifact 的大小和 retention 必须有默认上限；长期保留必须有用途、责任人和复核入口。
- 性能实验失败、超时或只覆盖局部 workload 时，只记录诊断结论，不推广为整体收益。
- 定期检查 required checks、触发规则、缓存命中率、Artifact 体积和保留量，防止优化策略随仓库演进失效。

## Current repository mapping

当前仓库的实现可作为以下通用原则的案例：

- Backend archive producer 与 resource profiles：一次构建、多档位回放。
- 前端测试迁移：将纯语义覆盖下沉到 Vitest，同时保留真实浏览器边界。
- Stateful SQLite fixture：复用当前 schema 模板，同时保留 migration、文件、压缩和锁竞争测试。
- 性能 A/B workflow：使用不可变候选、完整 workload 和成本指标，避免从局部收益下整体结论。
- Quality gate contract：确保必需检查、失败传播和 workflow 拓扑保持一致。

## References

### Repository

- `.github/workflows/ci-pr.yml`
- `.github/workflows/ci-main.yml`
- `.github/workflows/ci-observability-performance.yml`
- `.github/scripts/run-backend-tests.sh`
- `.github/scripts/check_quality_gates_contract.py`
- `docs/specs/q7yt7-backend-test-resource-profiles/SPEC.md`
- `docs/solutions/performance/rust-backend-test-runtime-feedback-loop.md`
- `docs/solutions/workflow/main-branch-protection-quality-gates.md`
- `docs/solutions/workflow/scoped-git-hooks-and-fingerprinted-worktree-bootstrap.md`

### Historical cases

- [PR #1178: migrate redundant browser coverage](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1178)
- [PR #1182: streamline frontend validation](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1182)
- [PR #1192: optimize Stateful SQLite fixture setup](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1192)

### GitHub documentation

- [Billing and usage](https://docs.github.com/en/actions/concepts/billing-and-usage)
- [GitHub Actions billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions)
- [Store and share data with workflow artifacts](https://docs.github.com/en/actions/tutorials/store-and-share-data)
- [Removing workflow artifacts](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/remove-workflow-artifacts)
- [Concurrency](https://docs.github.com/en/actions/concepts/workflows-and-actions/concurrency)
