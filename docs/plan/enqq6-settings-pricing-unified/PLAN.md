# 统一设置页：代理配置 + 价格配置（替换旧方案）

## Goal

将当前“代理模型弹窗 + 文件价目表”替换为独立设置页与数据库持久化配置，使线上可直接维护价格并让后续新流量成本统计即时生效。

## In / Out

### In

- 新增独立设置页 `/settings`，包含“代理配置”“价格配置”两个区块。
- 后端新增统一设置接口：`GET /api/settings`、`PUT /api/settings/proxy`、`PUT /api/settings/pricing`。
- 移除旧接口：`GET/PUT /api/settings/proxy-models`。
- 价格配置改为 SQLite 持久化并支持在线更新（热生效）。
- 预置已知模型价格（OpenAI standard 口径），并将 `gpt-5.3-codex` 先按 `gpt-5.2-codex` 价格预置为 temporary。
- 前端自动保存：代理即时保存；价格变更 `600ms` debounce 自动保存，失焦立即提交。

### Out

- 不做历史调用成本回填。
- 不做模型别名或模糊映射（仅精确 model id）。
- 不扩展代理底层运行参数可视化（例如 upstream URL、timeout 等）。

## Acceptance Criteria

1. Given 首次部署新版本，When 打开 `/settings`，Then 页面展示代理配置与预置价格表，且可编辑。
2. Given 修改价格并保存，When 产生新代理调用，Then 新记录成本按新价格估算。
3. Given 旧入口被替换，When 访问旧接口 `/api/settings/proxy-models`，Then 返回 404。
4. Given 自动保存失败，When 前端收到错误，Then 显示错误并回滚到最近服务端快照。
5. Given 系统已有历史记录，When 更新价格，Then 历史记录成本不发生回写变化。

## Testing

- Backend: `cargo test`（覆盖 settings API、持久化、代理模型行为与价格热更新）。
- Backend: `cargo check`。
- Frontend: `cd web && npm run test`。
- Frontend: `cd web && npm run build`。
- E2E: 更新并执行设置相关 Playwright 用例。

## Risks

- 自动保存频繁写入可能导致抖动：通过 debounce 与失败回滚控制。
- 价格预置与实际账单存在偏差：通过 UI 可编辑与 temporary 标记缓解。
- 移除旧接口可能影响外部脚本：在 README 明确 breaking change。

## Milestones

- [x] M1 requirements freeze 与 docs/plan 索引更新
- [x] M2 后端新 settings API + pricing DB 持久化
- [x] M3 前端 `/settings` 页面与自动保存
- [x] M4 测试通过、PR 创建并完成 checks 跟踪

## 变更记录 / Change log

- 2026-02-23: 初始化计划并冻结范围与验收标准。
- 2026-02-23: 完成旧方案替换（`/api/settings/proxy-models` 下线、`/settings` 上线、价目表改为 SQLite 持久化并可在线编辑）。
- 2026-02-23: 创建 PR #47，完成本地验证（cargo test / web lint+build / settings e2e）并跟踪 CI Pipeline #142 通过。
