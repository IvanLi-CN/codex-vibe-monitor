# `main` 主干保护与 quality-gates 对齐

## 适用场景

- 仓库要求 `main` 只能通过 PR 合并，不能被任何高权限身份直接 push。
- PR 必须在全部治理检查通过后才能进入可合并态。
- GitHub live ruleset 与仓库内 `.github/quality-gates.json` 需要长期保持一致。

## 核心结论

- 主干保护不能只靠仓库文档或本地流程约定，必须同时锁定 GitHub ruleset。
- `required_checks` 应直接表达“PR 合并前必须全绿”的完整集合，不再为 PR gate 保留 informational 例外。
- live quality-gates 脚本只能在 ruleset payload 明确返回 `bypass_actors` 时宣称“已验证 bypass 为空”；拿不到该字段时只能标记为 `unverified`，不能做假阳性结论。

## 推荐配置

### GitHub `main` ruleset

- `Require a pull request before merging`
- `Do not allow bypassing the above settings` / `Include administrators`
- `bypass actors = empty`
- `Require status checks to pass before merging`
- `Require branches to be up to date before merging`
- `Require signed commits`
- `Disallow force pushes`
- `Disallow branch deletions`

### `required_checks`

- `Validate PR labels`
- `Lint & Format Check`
- `Front-end Tests`
- `Records Overlay E2E`
- `Backend Tests`
- `Build Artifacts`
- `Review Policy Gate`

## 实施要点

- `.github/quality-gates.json` 与 fixtures 必须同步更新，避免 contract self-test 通过而 live fixtures 仍停留在旧规则。
- `check_quality_gates_contract.py` 应明确禁止为 PR gate 保留 `informational_checks`。
- `check_live_quality_gates.py` 的输出需要单独带出 `bypass_actor_status`，方便区分 `verified` 与 `unverified`。

## 常见坑

- 只把 required checks 写进文档，没有同步 GitHub ruleset，结果高权限用户仍可直推 `main`。
- 把 `Front-end Tests`、`Records Overlay E2E` 留在 informational，导致“PR 检查全过”只是口头说法，不是服务端硬门。
- live 脚本把缺失的 `bypass_actors` 当成空数组，误报“已验证没有 bypass”。
