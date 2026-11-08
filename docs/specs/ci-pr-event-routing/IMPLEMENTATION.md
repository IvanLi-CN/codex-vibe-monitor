# PR CI 事件路由实现

生产工作流的启用必须分两步：先部署同时接受完整静态格式与完整元数据包装格式的校验器，保持实际调度不变；兼容校验器进入 base 后，再为各 job 启用事件守卫和独立检查名称。merge_group 始终从 base 读取可信校验器，不因本优化切换到候选脚本。

当前步骤保留 `.github/workflows/ci-pr.yml` 的静态格式。`.github/scripts/fixtures/quality-gates-contract/ci-pr.yml` 保存待启用格式；`.github/scripts/test-ci-pr-event-policy.mjs` 在启用前使用该 fixture，启用后使用生产工作流执行事件矩阵。校验器拒绝两种格式混用，并在恢复原合同前精确校验名称和守卫。

元数据条件在各 job 入口生效，没有新增分类 runner。GitHub 对 job-level 跳过任务可能显示未求值的名称表达式；两种平台展示都必须与原 required-check 名称分离。原有代码 job 的依赖与内部步骤保持不变。

验证入口：`bun .github/scripts/test-ci-pr-event-policy.mjs`、`bash .github/scripts/test-quality-gates-contract.sh`。托管平台正文编辑的实际结果在 PR 证据中记录，不写入源码以免改变已验证候选。

回退必须以完整格式为单位，同时恢复工作流、fixture 和机械合同；不得仅移除检查名称隔离。
