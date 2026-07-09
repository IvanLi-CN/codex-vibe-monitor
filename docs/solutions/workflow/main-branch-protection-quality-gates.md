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
- `Backend Tests (Lightweight)`
- `Backend Tests (Stateful SQLite)`
- `Backend Tests (Archive / File I/O)`
- `Build Artifacts`
- `Review Policy Gate`

## 实施要点

- `.github/quality-gates.json` 与 fixtures 必须同步更新，避免 contract self-test 通过而 live fixtures 仍停留在旧规则。
- `CI PR` 可以对 same-repo stacked PR 保持开启，但 `Label Gate` 与 `Review Policy` 仍应只对 `base=main` 的 PR 生效；这样 stacked PR 也有服务端 CI 证据，而 owner-facing merge policy 仍只绑定 `main`。
- live GitHub rules 对齐检查同样只应阻断 `base=main` 的 PR；stacked PR 可以复用当前分支 contract 自检，但不应要求 `main` 的服务端 ruleset 预先反映未合并的拓扑改动。
- `check_quality_gates_contract.py` 应明确禁止为 PR gate 保留 `informational_checks`。
- `check_live_quality_gates.py` 的输出需要单独带出 `bypass_actor_status`，方便区分 `verified` 与 `unverified`。
- 由上游 workflow 触发的发布链路应把上游失败转成显式 failed gate，而不是依赖后续 job 的 `if` 条件自然跳过；否则失败通知通常只监听 `failure`，不会覆盖 `skipped`。
- 当 PR 本身修改 quality-gates contract 包或受控 workflow 拓扑时，PR lint 需要一个同仓库限定的 current-branch self-validation 路径；否则 base branch 的旧 contract 会永久挡住新增受控 job。

## 常见坑

- 只把 required checks 写进文档，没有同步 GitHub ruleset，结果高权限用户仍可直推 `main`。
- 把 `Front-end Tests`、`Records Overlay E2E` 留在 informational，导致“PR 检查全过”只是口头说法，不是服务端硬门。
- live 脚本把缺失的 `bypass_actors` 当成空数组，误报“已验证没有 bypass”。
- `Release` 只在 `CI Main success` 时运行发布 job，但没有单独的失败 gate，导致上游失败时发布 run 显示为 `skipped` 且通知链路不报警。
- 新增受控 workflow job 时只更新目标 workflow，却不更新 PR trusted-source 自举规则，导致 PR 被 `main` 上旧 contract 判定为 drift。
