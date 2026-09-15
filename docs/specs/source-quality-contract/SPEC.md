# 全源码结构质量合同

> 本文是源码结构质量的长期需求合同。当前实现覆盖与 rollout 事实记录在 `IMPLEMENTATION.md`，主题生命周期记录在 `HISTORY.md`。

## Context and Scope

- Context: 仓库包含 Rust backend、Web/Docs TypeScript、测试、Story/demo 与维护脚本；现有格式和 lint 检查没有阻止结构复杂度与超长文件持续增长。
- In scope: 所有被 Git 跟踪的手写 `.rs`、`.ts`、`.tsx`、`.js`、`.jsx`、`.css`、`.py`、`.sh` 源码，以及它们的本地检查器、baseline ratchet、hooks 和 CI 质量门。
- Out of scope: 运行时业务语义、公开 API、数据库 schema、发布版本、镜像构建内容和第三方/生成物本身。

## Terms and Interfaces

- `Source Structure Check`: 读取 tracked source 的多语言 AST 检查器，输出结构边界和文件预算诊断。
- `Biome`: Web/Docs 的主要 formatter 和 linter；其 JSON diagnostics 由结构质量 wrapper 统一执行 ratchet。
- `ratchet`: 有初始 baseline 时只允许违规身份消失或度量下降，不允许新增身份或度量上升。
- `zero`: 不读取 baseline，任何结构或 lint 诊断都使检查失败。
- Interface: `tools/source-structure-check` CLI、`scripts/check-source-quality.sh`、`scripts/check-staged-source-quality.sh`、Lefthook hooks 与 `Source Structure Check` required job。

## Requirements

### REQ-SQC-001

- 系统 MUST 从 `git ls-files` 的 tracked 文件集合发现 `.rs`、`.ts`、`.tsx`、`.js`、`.jsx`、`.css`、`.py`、`.sh`，并默认检查每一个匹配项。
- 系统 MUST 只排除具有可复现生成来源且在 scope 配置中逐项声明的路径；初始唯一排除项是 `web/public/mockServiceWorker.js`。构建目录、依赖目录和未跟踪文件不得成为隐藏源码的排除理由。
- Outputs: 检查输入集合和排除集合均可在诊断中复核。

### REQ-SQC-002

- 系统 MUST 对 Rust、TypeScript、TSX、JavaScript、Python、Shell 函数执行 `<=100` 物理行、`<=7` 形式参数和 `<=4` 层条件/循环/分支/异常嵌套限制。
- 系统 MUST 限制生产文件为 `<=1000` 行，独立 test/story/demo 文件为 `<=2000` 行，Shell/Python 工具为 `<=500` 行。
- 系统 MUST 限制 `src/main.rs`、`web/src/main.tsx`、`web/src/App.tsx`、`docs-site/rspress.config.ts`、脚本/CLI 入口和 checker 入口为 `<=500` 行且不包含 inline tests。

### REQ-SQC-003

- Source Structure Check MUST 使用 `syn` 解析 Rust，使用对应 Tree-sitter grammar 解析 TypeScript/TSX/JavaScript、Python 和 Shell，并以 Biome 解析 CSS。
- 解析失败 MUST 直接使检查失败，不能作为 baseline 或 waiver 保存。
- Diagnostics MUST 使用稳定的 `rule/path/subject/actual/limit` 字段，方便 staged、全量和 CI 结果比较。

### REQ-SQC-004

- baseline 模式 MUST 记录诊断身份和度量值，并拒绝新诊断身份及任一度量值增长。
- baseline guard MUST 只允许首次创建和从 `ratchet` 到 `zero` 的单向转换；不得扩充、重写或从 `zero` 回退。
- `zero` 模式 MUST 删除 baseline 文件，并在任何诊断存在时失败；最终状态不得有 waiver。

### REQ-SQC-005

- Rust quality gate MUST 使用 `too-many-lines=100`、`too-many-arguments=7`、`excessive-nesting=4` 和 `-D warnings`。
- 过渡期间 Source Structure Check MUST 实施与上述 Clippy 阈值等价的 AST 规则；所有 Rust `allow`、`expect` 或嵌套 `cfg_attr` 对这三项结构 lint 的压制 MUST 判为违规。
- Biome MUST 保持 Web/Docs 主 lint；其 JSON diagnostics MUST 经过同一 ratchet，最终所有 warning/error MUST 为零并升格为 error。

### REQ-SQC-006

- pre-commit MUST 按顺序检查 staged Biome 和 Source Structure Check；pre-push MUST 检查全量 Biome、Source Structure Check 和 Rust Clippy。
- PR 与 main MUST 提供全量 `Source Structure Check` required job；Integration CI MUST 只响应 `prd/source-quality-contract` 的 push，使用 `contents: read`，且不得触发 release 或 image publication。
- Quality gate 变更 MUST 同步 `.github/quality-gates.json`、workflow contract fixtures 和 hook contract tests。

### REQ-SQC-007

- remediation MUST 通过独立可验收的 child PR 按生产、测试、Story/demo 和工具边界推进；不得用单个巨型重写代替连续收敛。
- Initiative 完成时 MUST 删除全部 baseline 记录、清除所有三项结构 lint suppression、修复全部 Biome diagnostics 和超限源码，并以 aggregate PR 的全量验证证明 zero 状态。

## Verification

### VER-SQC-001

- Method: Source scope fixture and tracked-file inventory check.
- covers: `REQ-SQC-001`
- Pass condition: 所有匹配 tracked source 被检查，生成 worker 是唯一初始排除项，新增排除没有无依据路径。

### VER-SQC-002

- Method: 多语言 AST fixture suite with boundary values at limit, limit+1, and malformed input.
- covers: `REQ-SQC-002`, `REQ-SQC-003`
- Pass condition: 函数、嵌套、文件、入口和 inline-test 诊断与 `actual/limit` 完全一致，解析错误退出非零。

### VER-SQC-003

- Method: ratchet fixture suite covering first baseline, unchanged, decrease, new identity, metric increase, zero transition, and forbidden rollback.
- covers: `REQ-SQC-004`
- Pass condition: 只有允许的单向状态转换通过，最终 zero 对任一诊断失败。

### VER-SQC-004

- Method: Rust suppression fixture, Clippy threshold command, and Biome JSON diagnostic wrapper.
- covers: `REQ-SQC-005`
- Pass condition: 三项结构 suppression 和 Biome diagnostics 均不能绕过 gate，最终 Clippy 使用固定阈值和 `-D warnings`。

### VER-SQC-005

- Method: hook, PR/main, and branch-push workflow contract tests.
- covers: `REQ-SQC-006`
- Pass condition: staged/pre-push/PR/main 分层按约定执行，Integration CI 只运行在指定分支且无发布权限或发布步骤。

### VER-SQC-006

- Method: aggregate zero-state inventory and full repository validation.
- covers: `REQ-SQC-007`
- Pass condition: baseline、waiver、结构性 suppression、Biome diagnostics 和超限文件/函数均为零。

## Related ADRs

None

## References

- `./IMPLEMENTATION.md`
- `./HISTORY.md`
- `docs/agents/issue-tracker.md`
