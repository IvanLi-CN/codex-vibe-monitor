# Release 工作流 PR 版本评论（已撤销）

## 状态

- Status: 已撤销

## 当前结论

Release workflow 不再在发布完成后向来源 PR 写入版本、状态或结果评论，也不再申请用于 PR 评论的 GitHub API 写权限。

成功发布由负责该次 release 的 agent 直接向 owner 报告。Release 的 tag、GitHub Release、多架构镜像、发布队列和失败通知保持原有职责与行为。

本文保留在归档目录中，仅作为已撤销方案的索引记录；其中的评论、marker、评论 API 和评论权限不属于当前 workflow 合同。
