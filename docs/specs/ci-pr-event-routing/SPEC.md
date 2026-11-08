# PR CI 事件路由

## Context and Scope

本主题约束 PR 元数据编辑与代码验证之间的调度边界，减少标题、正文编辑造成的重复构建。应用行为、runner 容量及发布策略不在本主题范围内。

## Requirements

### REQ-CI-EVENT-001

PR 的 edited 事件携带 title 或 body 变更且无 base 变更时，CI PR MUST 跳过重型验证任务，仅允许 Repository Tooling Checks 使用一个轻量 runner；标签和 review 策略 MUST 继续由各自工作流验证。

### REQ-CI-EVENT-002

元数据路径 MUST 保留原检查名称和仓库必需检查规则，并由 Repository Tooling Checks 证明同一仓库、PR、head 和 base 的最近完整 CI 全部成功后，才允许合并。较新的完整运行失败、取消、尚未完成、证据缺失、过期或部分重跑 MUST NOT 退回复用较早成功。跳过重型任务本身 MUST NOT 构成完整代码验证证据。

### REQ-CI-EVENT-003

opened、reopened、synchronize、ready_for_review、merge_group、base 编辑，以及缺少已识别元数据字段的 edited 事件 MUST 保持完整验证路径；原任务依赖、门禁名称及生产者失败传播 MUST 保持原义。元数据编辑 MUST 不取消代码运行。

### REQ-CI-EVENT-004

首次启用 MUST 先将兼容校验器及可信证据读取器部署到 base，再修改生产工作流。兼容校验器 MUST 接受完整原格式、历史包装格式和完整单门禁格式，并拒绝混合格式、守卫、可信来源及权限漂移。merge_group 的可信合同校验器及元数据证据读取器 MUST 保持 base 来源。

### REQ-CI-EVENT-005

元数据路径 MUST 使用固定 runner、checkout 和隔离解释器，拒绝候选分支的启动环境、容器及 shell defaults 注入。元数据证据读取器 MUST 仅使用只读权限，从原生 Actions 运行读取事件证明和完整 job 集合。事件证明 MUST 绑定 run/attempt、head/base 和合成 merge commit；全部 17 个原始 job MUST 在同一 attempt 成功，完成时间均在七日内。下载附件 MUST 限制大小、成员和有效期，并且 MUST NOT 将 GitHub API 凭据转发至附件下载域名。接受证据前 MUST 再次核对 live PR 的 head/base、运行集合及源 attempt/status；证据集合变化 MUST 重新验证。较新 attempt 的执行时间 MUST 优先于原始运行编号，时间顺序不明 MUST 阻断复用。

## Verification

### VER-CI-EVENT-001

- covers: `REQ-CI-EVENT-001`, `REQ-CI-EVENT-003`
- 通过 GitHub 官方表达式解析器执行事件矩阵，确认代码事件保持 17 个任务，元数据编辑仅调度一个轻量证据 job，原有重型步骤不执行。

### VER-CI-EVENT-002

- covers: `REQ-CI-EVENT-002`, `REQ-CI-EVENT-004`
- 机械合同接受三种完整格式；逐 job 混用、守卫漂移、额外写权限和可信来源变更必须被拒绝；其余既有合同继续验证依赖和失败传播。

### VER-CI-EVENT-003

- covers: `REQ-CI-EVENT-001`, `REQ-CI-EVENT-002`
- 在真实 PR 上完整代码 CI 成功后修改正文，确认仅一个轻量 runner、16 个 skipped jobs，并确认 GitHub 实际合并门禁恢复可合并。CLI required checks 绿色不能单独证明验收通过。另以受控负例确认完整失败或缺失证据使必需证据 job 失败、合并门禁阻断。

### VER-CI-EVENT-004

- covers: `REQ-CI-EVENT-004`, `REQ-CI-EVENT-005`
- 离线对抗测试覆盖身份、attempt、较新失败、缺失证明、过期、部分重跑、merge parents、PR 变更竞态、附件成员及凭据隔离。启用前确认 base 已部署读取器，生产工作流在兼容阶段保持不变。

## Related ADRs

- [ADR 0054：PR 元数据 CI 证据门禁](../../adr/0054-pr-metadata-ci-evidence-gate.md)

## References

- [GitHub 必需状态检查排障](https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/troubleshooting-required-status-checks)
- [GitHub job 条件与跳过状态](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-jobs-with-conditions)
