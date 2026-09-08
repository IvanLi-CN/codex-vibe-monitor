# Release PR 评论权限补齐（已撤销）

## 状态

- Status: 已撤销

## 当前结论

Release workflow 不再向来源 PR 写入发布结果，因此不再需要 `issues: write` 或 `pull-requests: write`。发布 job 仅保留创建 tag、GitHub Release、镜像发布和 release queue 调度所需的权限。

成功发布由 release-owning agent 直接向 owner 报告。Release failure notification workflow 保持不变，发布失败仍按既有路径通知。

本文保留在归档目录中，仅作为已撤销方案的索引记录；历史评论权限问题不属于当前 workflow 合同。
