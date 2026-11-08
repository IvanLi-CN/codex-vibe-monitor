# PR CI 事件路由实现

上线分为兼容与启用两步。兼容阶段保持生产 `.github/workflows/ci-pr.yml` 不变，将可信读取器和新格式合同先部署到 base；fixture 演练未来工作流。启用阶段保留全部原检查名称，在 16 个重型 job 入口加元数据守卫，仅 Repository Tooling Checks 执行轻量证据路径。其代码路径仍执行原工具合同和检查。

轻量 job 从事件 base SHA 读取 `.github/scripts/ci_pr_metadata_evidence.py`，不执行候选分支中的读取器。完整和元数据运行均先上传一个绑定 run/attempt、PR、head/base 和 merge SHA 的微型 `event.json`，保留七日。元数据路径使用 `contents:read` 和 `actions:read`，不创建检查、不写状态、不修改必需检查规则。完整代码路径保持原十五分钟超时，元数据路径最多七十五分钟，其中证据等待最多一小时，等待期间门禁保持未完成。

读取器先核对 live PR，再按当前 attempt 的启动时间查找最近完整 CI；同秒无法确定先后的完整证据拒绝复用。具有有效元数据证明且其余 16 个任务全部 skipped 的运行可略过；无法分类的新运行阻断或等待，不回退到旧成功。完整源必须全部 17 个规范名称任务在同一 attempt 成功、各完成时间均在七日内、merge commit parents 精确包含 head/base，并在接受前重新核对 live PR 和运行集合快照。源 attempt、状态或运行集合变化时重新归约证据，不能沿用下载前的成功快照。部分重跑不能刷新旧证据。API、证明或附件异常一律阻止复用，需要完整 CI 恢复。

事件条件使用官方 `@actions/expressions` 执行矩阵测试；读取器的离线对抗测试由 `.github/scripts/test-ci-pr-metadata-evidence.py` 承担，接入既有 quality-gates self-tests。兼容校验器在恢复原合同前精确检查全部事件守卫、原步骤守卫、可信读取步骤、附件设置和只读权限。

托管平台验收必须同时取得实际合并门禁和 Actions 运行证据。成功路径确认一个 runner 与可合并状态；失败路径确认同一必需证据 job 阻断合并。当前兼容提交仅证明准备阶段，不能宣称优化已生效。回退以完整格式为单位恢复原静态工作流，保留完整验证路径。
