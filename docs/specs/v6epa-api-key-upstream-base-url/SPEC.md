# API Key 账号上游地址支持（#v6epa）

## 状态

- Status: 已完成
- Created: 2026-03-15
- Last: 2026-03-15

## 背景

- 当前号池 API Key 账号只能保存名称、分组、限额与备注，无法给单个账号指定独立上游地址。
- 池路由请求始终使用全局 `OPENAI_UPSTREAM_BASE_URL`，导致“不同 API Key 账号走不同上游”的运营需求无法落地。
- 现有创建页和详情编辑页都没有该字段，数据库也缺少持久化列，因此即使临时拼接前端输入也无法可靠回读或参与转发。

## 目标 / 非目标

### Goals

- 为 API Key 账号新增可选 `upstreamBaseUrl`，覆盖创建、详情读取与详情编辑。
- 在 SQLite `pool_upstream_accounts` 持久化账号级上游地址，留空时明确回退全局 `OPENAI_UPSTREAM_BASE_URL`。
- 让池路由真正使用账号级上游地址构造目标 URL，并在重定向头重写时保持同一上游基址语义。
- 对非法 URL 做显式校验，避免把脏值写入数据库或运行时才失败。

### Non-goals

- 不为 OAuth / 批量 OAuth 账号增加账号级上游地址输入。
- 不更改 `UPSTREAM_ACCOUNTS_USAGE_BASE_URL`、OAuth issuer/client id、forward proxy 节点管理或全局 Settings 页。
- 不在列表表格新增上游地址列；本轮仅要求创建页与详情编辑页可维护。

## 功能规格

### 数据与接口

- `pool_upstream_accounts` 新增 nullable `upstream_base_url TEXT`，字段值保存标准化后的绝对 URL 字符串。
- `POST /api/pool/upstream-accounts/api-keys` 新增可选 `upstreamBaseUrl`；为空或缺省表示不覆写。
- `PATCH /api/pool/upstream-accounts/:id` 新增可选 `upstreamBaseUrl`；支持新增、更新与清空。
- `GET /api/pool/upstream-accounts/:id` 对 API Key 账号返回 `upstreamBaseUrl`，供详情页回显。

### 运行时生效规则

- 当池路由选中 API Key 账号且该账号设置了 `upstreamBaseUrl` 时，请求目标 URL 必须基于该账号上游地址 + 原始 path/query 构造。
- 当账号未设置 `upstreamBaseUrl` 时，继续使用全局 `OPENAI_UPSTREAM_BASE_URL`，既有行为不变。
- 响应 `Location` 头的归一化必须基于“本次实际使用的上游基址”，避免账号级覆写后仍按全局基址回写。
- OAuth 账号即使未来数据库列存在，也不读取/不写入该字段；运行时继续走全局上游地址。

### 校验与兼容

- 后端接收 `upstreamBaseUrl` 时必须去首尾空白；空字符串按 `None` 处理。
- 非空值必须能解析为绝对 URL，且保留路径前缀拼接语义，与现有全局 `OPENAI_UPSTREAM_BASE_URL` 一致。
- 旧数据库通过 schema ensure 自动补列，不引入单独 migration 框架。

## 验收标准

- Given 用户在 API Key 创建页填写合法 `upstreamBaseUrl`，When 保存成功并打开详情，Then 详情编辑表单可回显同一值。
- Given 用户清空已存在 API Key 账号的 `upstreamBaseUrl`，When 保存并刷新详情，Then 字段为空且运行时回退全局上游地址。
- Given 某 API Key 账号设置了 `upstreamBaseUrl`，When 请求被池路由分配到该账号，Then 实际上游请求与 `Location` 头处理都使用该账号基址。
- Given 用户提交非法 URL，When 创建或更新 API Key 账号，Then 后端返回明确错误且数据库不写入脏值。
- Given 用户位于 OAuth / 批量 OAuth 创建流，When 打开表单，Then 不会出现 `upstreamBaseUrl` 输入。

## 质量门槛

- `cargo fmt`
- `cargo check`
- `cargo test`
- `cd web && bun run test`
- `cd web && bun run build`
- 浏览器 / 组件级 smoke：API Key 创建页与详情编辑页的上游地址输入、保存与回显

## 变更记录

- 2026-03-15: 创建增量 spec，冻结 API Key 账号级上游地址的数据模型、运行时生效规则与验收口径。
- 2026-03-15: 完成 SQLite 补列、API Key 创建/详情编辑接线、池路由账号级上游地址生效，并通过本地 Rust/Web 验证。
