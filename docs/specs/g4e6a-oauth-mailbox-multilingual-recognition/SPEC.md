# OAuth 邮件多语言验证码与邀请识别（#g4e6a）

## 状态

- Status: 进行中
- Created: 2026-03-24
- Last: 2026-03-24

## 背景 / 问题陈述

- `3n287` 已把 MoeMail 邮箱会话、验证码摘要和邀请摘要接进 OAuth 创建流，但当前后端解析仍主要依赖英文模板。
- 真实 OpenAI 邮件已经出现中文等本地化主题和正文，例如“你的 OpenAI 代码为 438211”“输入此临时验证码以继续”，导致页面无法产出 `latestCode`。
- `m7a9k` 已把手动附着邮箱也纳入同一增强链路，因此解析修复必须同时覆盖生成邮箱与 attached 邮箱，不得改动现有前后端契约。

## 目标 / 非目标

### Goals

- 将 OAuth 邮件解析从“固定英文 regex”升级为“文本归一化 + 数字候选提取 + 语义判定”的后端管线。
- 在不改 HTTP / TS 字段的前提下，补齐多语言验证码识别与多语言邀请识别，覆盖 subject、plain text body 与 HTML stripped text。
- 支持全角数字、混合语言和本地化验证码/邀请文案，同时保持英文基线不回退。
- 补齐 Rust 回归测试，覆盖正例和负例，避免把无关数字或普通工作区链接误判成验证码 / 邀请。

### Non-goals

- 不扩展 MoeMail 响应字段，不依赖 sender/from 域名校验。
- 不新增邮件历史页、前端字段、额外按钮或新的 OAuth 交互步骤。
- 不把任何 4-8 位数字都视为验证码；必须保留语义门禁。
- 不改动 `OauthMailboxStatus`、`OauthInviteSummary` 或前端消费协议。

## 范围（Scope）

### In scope

- `src/upstream_accounts/mod.rs`：邮箱文本归一化、验证码候选提取、邀请语义识别与相关单元测试。
- `docs/specs/g4e6a-oauth-mailbox-multilingual-recognition/SPEC.md` 与 `docs/specs/README.md`：记录本增量的边界、验收和 fast-track 状态。

### Out of scope

- `web/src/lib/api.ts` 与前端类型契约。
- MoeMail API、数据库 schema、状态轮询接口字段或邀请复制字段形态。
- 基于发件人地址的品牌校验或第三方邮件供应商扩展。

## 需求（Requirements）

### MUST

- 后端在解析 subject、content、html 前，必须至少完成 HTML 去标签、Unicode 空白折叠与全角数字归一化。
- 验证码提取必须以“4-8 位数字候选 + 验证码语义 + OpenAI/ChatGPT 品牌或强验证码上下文”联合判定，不能只靠裸数字命中。
- 邀请提取必须要求邀请语义与 CTA 链接同时成立；普通工作区说明链接或概览邮件不得误判为 invite。
- 现有英文模板、中文本地化模板和混合语言模板都必须通过同一后端解析入口，不允许前端特判 locale。

### SHOULD

- 验证码弱语义（如 `code is` / `代码为`）只有在同一邮件具备 OpenAI/ChatGPT 品牌上下文时才命中。
- 邀请识别优先使用 OpenAI/ChatGPT 品牌上下文与 `workspace / invite / accept` 型链接联合判定，降低误判率。
- 新增测试应覆盖负例：OpenAI 普通通知、账单/订单号、以及非邀请性质的 workspace 链接。

## 功能与行为规格（Functional/Behavior Spec）

### Core flows

- `parse_mailbox_code(...)`：
  - 先汇总 subject/content/html 的标准化文本上下文；
  - subject 优先提取验证码；
  - 若 subject 未命中，再回退到 content、html；
  - 对每个 4-8 位候选数字按周边窗口检查“强验证码语义”或“品牌 + 弱验证码语义”，只接受满足门禁的候选。
- `parse_mailbox_invite(...)`：
  - 先检查 subject/body 是否存在邀请语义；
  - 再从 content/html 中提取 `workspace / invite / accept` 型链接；
  - 只有邀请语义与品牌上下文同时成立时，才写入现有 `invite-link` 摘要。

### Edge cases / errors

- 全角数字（如 `４３８２１１`）必须在归一化后稳定解析成 ASCII 数字。
- 纯数字订单号、账单号、普通 OpenAI 收据邮件，即使带 4-8 位数字也不能命中验证码。
- 普通 workspace 概览或帮助链接，即使 URL 包含 `workspace` 也不能在缺少邀请语义时命中 invite。

## 接口契约（Interfaces & Contracts）

- 对外接口保持不变：继续复用现有 `OauthMailboxStatus.latestCode`、`OauthInviteSummary` 与 `invite-link` copy label。
- 本增量仅改变 `src/upstream_accounts/mod.rs` 内部解析实现，不新增 HTTP 字段或前端状态字段。

## 验收标准（Acceptance Criteria）

- Given 中文主题如“你的 OpenAI 代码为 438211”，When 状态刷新解析该邮件，Then `latestCode.value` 为 `438211` 且 `source=subject`。
- Given 主题不含验证码但 HTML 中包含“输入此临时验证码以继续：４３８２１１”，When 状态刷新解析该邮件，Then `latestCode.value` 为 `438211` 且可记录为 `html` 来源。
- Given 非英文邀请邮件包含“邀请你加入 OpenAI 工作区”与 `https://chatgpt.com/workspace/invite/...`，When 状态刷新解析该邮件，Then 返回现有 `invite` 摘要字段。
- Given OpenAI 收据或普通工作区说明邮件包含 4-8 位数字或 workspace 链接，When 状态刷新解析该邮件，Then 不会误写 `latestCode` 或 `invite`。
- Given 既有英文验证码与邀请测试，When 运行回归，Then 继续通过且“最新消息优先”逻辑不变。

## 实现前置条件（Definition of Ready / Preconditions）

- 已确认当前 MoeMail message detail 仅包含 `subject`、`content`、`html` 与 `receivedAt`
- 已确认本轮不新增 sender/from 字段，因此品牌判定只能依赖现有正文、主题与链接
- 已确认前端消费契约保持不变，仅允许后端内部解析升级

## 非功能性验收 / 质量门槛（Quality Gates）

### Testing

- `cargo test parse_mailbox -- --nocapture`
- `cargo test normalize_mailbox_text -- --nocapture`
- `cargo check`
- `cargo test`
- `cd web && bun run test`
- `cd web && bun run build`

### Quality checks

- fast-track PR 收敛期间必须执行 spec-sync，确保 spec 与最终行为一致

## 文档更新（Docs to Update）

- `docs/specs/README.md`: 新增索引并在 PR / merge 事实确定后同步状态与备注

## 实现里程碑（Milestones / Delivery checklist）

- [x] M1: 后端新增邮箱文本归一化、全角数字归一化与语义候选提取辅助逻辑。
- [x] M2: 多语言验证码与邀请识别改为共享解析管线，不再只依赖英文模板。
- [x] M3: Rust 单测覆盖中文 subject、HTML + 全角数字、本地化 invite 与负例。
- [ ] M4: fast-track 完成本地验证、PR、review-loop、merge 与 cleanup，并同步 spec 最终状态。

## 方案概述（Approach, high-level）

- 保留现有 `parse_mailbox_code` / `parse_mailbox_invite` 入口，避免扩散接口变更。
- 通过轻量归一化把 ASCII 大写、全角字符和空白差异收口后，再做候选数字与关键词判定。
- 邀请识别继续输出现有 `invite-link`，但把门禁升级为“邀请语义 + CTA 链接 + 品牌上下文”联合成立。

## 风险 / 开放问题 / 假设（Risks, Open Questions, Assumptions）

- 风险：缺少 sender/from 字段时，极少数不带品牌文本的验证码邮件可能仍有漏判；本次通过“强验证码语义可单独成立”降低该风险。
- 风险：某些 invite 邮件若只给跳转包装链接且不暴露 `workspace/invite/accept` 语义，当前规则可能保守不识别。
- 需要决策的问题：None
- 假设（需主人确认）：当前 OpenAI 邀请邮件仍会在正文或链接中暴露 workspace / invite / accept 语义。

## 变更记录（Change log）

- 2026-03-24: 新建 follow-up spec，冻结 OAuth 邮件多语言验证码与邀请识别的后端解析边界、误判门禁与质量门槛。
- 2026-03-24: 完成后端解析升级与 Rust 回归测试补齐，等待 fast-track 的本地全量验证、PR 收敛与 merge cleanup。
- 2026-03-24: 本地 `cargo check`、`cargo test`、`cd web && bun run test` 与 `cd web && bun run build` 已通过，进入 fast-track 的 PR / review / merge 收口阶段。
- 2026-03-24: 创建 PR #215，并根据 review 收紧 invite CTA 链接判定，避免把普通 workspace 页面误存成邀请链接。
- 2026-03-24: 根据后续 review 继续收紧 body-only invite 判定，要求 workspace 语义与真实邀请 CTA 同时成立，并排除帮助文档类链接。
- 2026-03-24: 根据后续 review 恢复 query 型邀请 CTA 识别（如 `?invite=` / `?accept=`），避免误伤合法邀请链接。

## 参考（References）

- `docs/specs/3n287-oauth-temp-mail-automation/SPEC.md`
- `docs/specs/m7a9k-oauth-manual-mailbox-attach/SPEC.md`
- `src/upstream_accounts/mod.rs`
