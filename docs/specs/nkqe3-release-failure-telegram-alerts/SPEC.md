# Release 失败 Telegram 告警接入

## 状态

- Spec ID: `nkqe3`
- State: `active`
- Status: `已完成`
- Scope: 为 `Release` workflow 接入失败告警与 repo-local smoke test

## 目标

为 `codex-vibe-monitor` 接入共享的 Telegram 失败告警工作流，使真实 `Release` 失败能够通过 `workflow_run` 自动告警，同时保留一个安全的 repo-local `workflow_dispatch` smoke test 入口。

## 范围

### In scope

- 新增 `.github/workflows/notify-release-failure.yml`
- 监听 `Release` workflow 的失败结果并转发到 `IvanLi-CN/github-workflows`
- 复用单个 repo secret：`SHOUTRRR_URL`
- 提供 repo-local `workflow_dispatch` smoke test
- 将该仓库作为首个真实发布失败验证目标

### Out of scope

- 不修改现有 `Release` 发布逻辑
- 不新增第二通知渠道
- 不改动 Docker / release snapshot / versioning 规则

## 需求

### Must

- 当 `main` 上的 `Release` workflow 以 `failure` 结束时，必须触发 notifier workflow
- notifier 必须显式调用 `IvanLi-CN/github-workflows/.github/workflows/release-failure-telegram.yml@main`
- notifier 必须显式传入 `secrets.SHOUTRRR_URL`
- notifier 必须保留 `workflow_dispatch` 入口，用于安全 smoke test
- 首次 rollout 必须完成一次真实 `Release` 失败验证，且失败发生在发布前校验阶段

## 验收标准

- Given `SHOUTRRR_URL` 已配置
  When 手动运行 `notify-release-failure.yml`
  Then Telegram 收到 smoke-test 消息
- Given `Release` 被手动触发且传入无效 `commit_sha`
  When `Release` 在 `main` 上的校验目标 SHA 阶段失败
  Then `notify-release-failure.yml` 通过 `workflow_run` 自动发送 Telegram 告警
- Given 某个非 `main` 分支手动触发 `Release`
  When 该 run 失败
  Then 不发送 production-style 失败告警
- Given `Release` 成功
  When notifier workflow 评估事件
  Then 不发送失败告警

## 变更记录

- 2026-04-11: 首次为 `codex-vibe-monitor` 接入共享 Telegram 发布失败告警与 repo-local smoke test。
