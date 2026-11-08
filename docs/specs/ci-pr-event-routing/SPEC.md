# PR CI 事件路由

## Context and Scope

本主题约束 PR 元数据编辑与代码验证之间的调度边界，减少标题、正文编辑造成的重复构建，同时保持代码检查身份、base 重验和失败传播。应用行为、runner 容量及发布策略不在本主题范围内。

## Requirements

### REQ-CI-EVENT-001

PR 的 edited 事件在携带 title 或 body 变更且未携带 base 变更时，CI PR MUST 跳过重型验证任务；标签和 review 策略 MUST 继续由各自工作流验证。

### REQ-CI-EVENT-002

元数据路径 MUST 使用与代码 required checks 不同的检查名称，不能用跳过结果覆盖既有失败、取消或运行中的代码检查。

### REQ-CI-EVENT-003

opened、reopened、synchronize、ready_for_review、merge_group、base 编辑，以及缺少已识别元数据字段的 edited 事件 MUST 保持完整验证路径；原任务依赖、门禁名称及生产者失败传播 MUST 保持原义。元数据编辑 MUST 不取消代码运行。

## Verification

### VER-CI-EVENT-001

- covers: `REQ-CI-EVENT-001`, `REQ-CI-EVENT-002`, `REQ-CI-EVENT-003`
- 通过 GitHub 官方表达式解析器运行事件矩阵，核对执行条件及检查身份；正常代码事件保持完整任务集合，元数据编辑不生成原 required-check 名称。

### VER-CI-EVENT-002

- covers: `REQ-CI-EVENT-002`, `REQ-CI-EVENT-003`
- 机械合同拒绝隔离名称或事件守卫漂移；其余既有 workflow 合同继续验证依赖和失败传播。

### VER-CI-EVENT-003

- covers: `REQ-CI-EVENT-001`, `REQ-CI-EVENT-002`
- 托管平台上完整代码验证通过后修改 PR 正文，确认新 CI PR 运行仅产生独立名称的 skipped jobs，且不消耗重型任务 runner、不改变原代码检查结果。

## Related ADRs

None

## References

- [GitHub job 条件与跳过状态](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-jobs-with-conditions)
- [GitHub job name 可用表达式上下文](https://docs.github.com/en/actions/reference/workflows-and-actions/contexts)
