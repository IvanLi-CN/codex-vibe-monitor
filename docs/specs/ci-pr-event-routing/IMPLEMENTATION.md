# PR CI 事件路由实现

`.github/workflows/ci-pr.yml` 在各 job 入口使用相同元数据条件，同时隔离 metadata 路径的检查名称。没有新增分类 runner；原有代码 job 的依赖与内部步骤保持不变。

`.github/scripts/check_quality_gates_contract.py` 先校验精确名称和守卫包装，再检查原有 job 合同。`.github/scripts/test-ci-pr-event-policy.mjs` 使用 `@actions/expressions` 执行事件矩阵与检查名称隔离测试。

验证入口：`bun .github/scripts/test-ci-pr-event-policy.mjs`、`bash .github/scripts/test-quality-gates-contract.sh`。托管平台正文编辑的实际结果在 PR 证据中记录，不写入源码以免改变已验证候选。

回退时必须同时恢复工作流、fixture 和机械合同；不得仅移除检查名称隔离。
