# Bun-first 工具链收敛 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/tr4ev-bun-first-toolchain/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.
- Biome owns the two JavaScript/TypeScript/CSS surfaces from the repository root while dprint remains the Markdown formatter; generated MSW worker output is excluded and Tailwind directives use parser support rather than blanket lint suppression.
- The required `Lint & Format Check` job name is preserved while Clippy is promoted from an advisory local check to `--all-targets --all-features -D warnings` in CI and pre-commit.

## Migrated Change History

## 变更记录（Change log）

- 2026-07-12: 根目录引入 Biome `2.5.3`，移除 web ESLint 配置与依赖，统一 `web/`、`docs-site/` lint/format/import organization 入口；完成全量前端格式化和阻断诊断收敛。
- 2026-07-13: 对 unsafe Hook 自动修复引发的行为回归完成测试收敛；保留 recommended 规则报告，将现存行为敏感诊断以 warning 呈现，并把历史 Hook 例外迁移为带理由的文件级 Biome suppression。
- 2026-07-12: `Lint & Format Check` 和 pre-commit 改为严格 Clippy，清理 Rust 诊断并验证 `cargo clippy --locked --all-targets --all-features -- -D warnings`；required-check 名称与 quality-gates contract 保持不变。

- 2026-06-23: GitHub Actions 与本地 / 容器 toolchain 基线刷新为 Bun `1.3.14`、Rust `1.96.0`、x64 `ubuntu-24.04`、arm `ubuntu-24.04-arm`，并将受控 action majors 升级到 `checkout@v7`、`cache@v5`、`github-script@v9`、`upload-artifact@v7`、`download-artifact@v8`、`configure-pages@v6`、`upload-pages-artifact@v5`、`deploy-pages@v5`、`setup-buildx-action@v4`、`login-action@v4`、`build-push-action@v7`。
- 2026-03-12: 创建 spec，冻结“Bun-first direct execution surface”定义、允许残留项与 PR 阶段 Docker smoke 要求。
- 2026-03-12: 仓库根与 `web/` 已迁移到文本 `bun.lock`，`package-lock.json` 删除，`README.md`、`AGENTS.md`、`lefthook.yml`、`Dockerfile`、`.github/workflows/ci.yml` 全部切换到 Bun-first 入口。
- 2026-03-12: 新增 `/.github/scripts/check-bun-first.sh` 作为运营面守门；本地已通过 `bun install --frozen-lockfile`（root + web）、`cargo fmt --all -- --check`、`cargo check --locked --all-targets --all-features`、`cargo test --locked --all-features`、`cd web && bun run lint`、`cd web && bun run test`、`cd web && bun run build`、`cd web && bun run build-storybook`，并在 shared testbox 完成 Docker smoke。
- 2026-03-12: PR #115 已创建并打上 `type:skip` / `channel:stable`；GitHub required checks 通过，`spec_drift_check.sh --base-ref origin/main --spec-path docs/specs/tr4ev-bun-first-toolchain/SPEC.md` 返回 `Spec同步状态=通过` / `Spec漂移=不存在`，`codex review --base origin/main` 未发现阻塞项。
