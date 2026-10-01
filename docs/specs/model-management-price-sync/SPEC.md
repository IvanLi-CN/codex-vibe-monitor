# 模型管理与价格同步

> 本 Spec 定义代理模型候选、预置成员与本地价格的长期行为。当前实现位置和 rollout 信息见 `IMPLEMENTATION.md`。

## Context and Scope

- Context: 模型预置名单与成本估算价格分散维护，更新报价需要人工逐项查找。
- In scope: 系统模型页、持久模型目录、预置成员开关、本地价格编辑与删除，以及按需从 models.dev 获取并人工选择价格。
- Out of scope: 自动价格验证、计费真值、定时同步、供应商专属运行时路由，以及估算器不支持的价格维度。

## Terms and Interfaces

- `model ID`: 代理请求、价格目录和模型候选共用的稳定键；一个 ID 只对应一个有效本地价格。
- `managed model`: 本地持久模型目录中可由用户设置代理预置状态的模型。
- `price candidate`: models.dev 按供应商提供、尚未应用到本地价格目录的报价。
- Interface: `/system/models` 页面；`/api/settings`、`/api/settings/proxy`、`/api/settings/pricing` 与模型目录同步、应用和删除 API；代理公开的 `GET /v1/models`。

## Requirements

### REQ-MODEL-MGMT-001

- The system MUST show the union of persistent managed models and locally priced models, keyed by model ID, with all supported token-price fields and `—` when no local price exists.
- Inputs: current model directory and local pricing catalog.
- Outputs: one editable row per model ID; manual price edits are marked `custom`.

### REQ-MODEL-MGMT-002

- The system MUST expose a preset Switch for every row and make enabled model IDs appear in the proxy-generated `GET /v1/models` response.
- Inputs: a managed model ID and its enabled state.
- Outputs: persisted preset membership reflected by the proxy model list; newly synchronized models start disabled.

### REQ-MODEL-MGMT-003

- Deleting a model MUST atomically remove its managed directory entry, local price, and preset membership without rewriting historical non-null invocation costs.
- Inputs: an existing model ID.
- Outputs: the model is absent after restart unless a later source sync adds it again, in which case its preset state is disabled.

### REQ-MODEL-MGMT-004

- The system MUST fetch models.dev only after a user opens synchronization preview; fetch and parse failures MUST leave local state unchanged and allow retry.
- Inputs: a user-triggered preview request.
- Outputs: retrieval state and provider/model price candidates, or a retryable error.

### REQ-MODEL-MGMT-005

- The synchronization preview MUST support provider-group selection and search across provider and model names; each new preview starts with every provider selected.
- Inputs: current provider groups, search text, and candidate catalog.
- Outputs: the visible candidate set scoped to selected groups and the search text.

### REQ-MODEL-MGMT-006

- The system MUST keep one local price per model ID; when selected providers offer multiple candidates for one ID, the user MUST choose exactly one provider candidate before applying it.
- Inputs: candidate quotes grouped by model ID.
- Outputs: a selected candidate whose displayed values are exactly the values written to the local price catalog.

### REQ-MODEL-MGMT-007

- Synchronization MUST import only input, output, cache-read, cache-write, and reasoning token prices; other priced dimensions MUST be identified as unsupported and skipped.
- Inputs: models.dev model cost fields.
- Outputs: only supported token prices become local estimates; unsupported dimensions remain visible in preview metadata.

### REQ-MODEL-MGMT-008

- The preview MUST default-select unambiguous new models and changed non-`custom` prices, leave changed `custom` prices unselected, and write nothing until the user applies the selected candidates.
- Inputs: provider-scoped candidates and the current local price source/value.
- Outputs: explicit selections; only selected candidates are applied and imported prices use source `models.dev`.

### REQ-MODEL-MGMT-009

- Synchronization MUST change prices and model-directory membership only; it MUST NOT enable proxy presets.
- Inputs: selected price candidates.
- Outputs: new managed models have preset state off; existing preset state remains unchanged.

### REQ-MODEL-MGMT-010

- The preview MUST use an exact official pricing-page URL only when the source supplies one; otherwise it MUST label the provider's supplied documentation URL as `供应商文档` and MUST NOT infer a pricing URL.
- Inputs: provider and model source metadata.
- Outputs: an accurately labeled external link or no link.

## Verification

### VER-MODEL-MGMT-001

- Method: stateful SQLite tests for migration, merged model listing, deletion, restart, and cost preservation.
- covers: `REQ-MODEL-MGMT-001`, `REQ-MODEL-MGMT-002`, `REQ-MODEL-MGMT-003`, `REQ-MODEL-MGMT-009`
- Pass condition: existing candidates and enabled state survive migration/restart; deletion remains deleted; rediscovery starts disabled; historical non-null costs are unchanged.

### VER-MODEL-MGMT-002

- Method: models.dev parser and synchronization API tests with catalog fixtures and failure cases.
- covers: `REQ-MODEL-MGMT-004`, `REQ-MODEL-MGMT-006`, `REQ-MODEL-MGMT-007`, `REQ-MODEL-MGMT-008`, `REQ-MODEL-MGMT-010`
- Pass condition: provider conflicts require an explicit choice; unsupported dimensions are excluded; custom prices default unchecked; preview failures write nothing; apply writes exactly selected values.

### VER-MODEL-MGMT-003

- Method: web unit tests, TypeScript typecheck, production build, and scenario-bound browser observation.
- covers: `REQ-MODEL-MGMT-001`, `REQ-MODEL-MGMT-002`, `REQ-MODEL-MGMT-004`, `REQ-MODEL-MGMT-005`, `REQ-MODEL-MGMT-006`, `REQ-MODEL-MGMT-008`, `REQ-MODEL-MGMT-010`
- Pass condition: the page and sync dialog support search, provider groups, state feedback, price editing, deletion, and selection at desktop/mobile sizes in light/dark themes without overlap.

## Related ADRs

- [ADR 0018: OpenAI GPT-6 pricing and usage semantics](../../adr/0018-openai-gpt-6-pricing-and-usage-semantics.md)
- [ADR 0023: Model management and manual price synchronization](../../adr/0023-model-management-and-manual-price-synchronization.md)

## Visual Evidence

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  requested_viewport: 1660x960
  viewport_strategy: storybook-viewport
  margin_policy: trim_only
  evidence_surface: page
  sensitive_exclusion: N/A
  submission_gate: approved
  story_id_or_title: System/SystemWorkspace/Models
  state: light desktop
  evidence_note: verifies the merged catalog, local token prices, missing-price marker, preset switches, and row actions
  image:
  ![模型页桌面浅色](./assets/models-page-desktop-light.png)

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  requested_viewport: 1660x960
  viewport_strategy: storybook-viewport
  margin_policy: trim_only
  evidence_surface: page
  sensitive_exclusion: N/A
  submission_gate: approved
  story_id_or_title: System/SystemWorkspace/ModelsDark
  state: dark desktop
  evidence_note: verifies the model catalog and controls in the dark theme
  image:
  ![模型页桌面深色](./assets/models-page-desktop-dark.png)

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  requested_viewport: 393x852
  viewport_strategy: storybook-viewport
  margin_policy: trim_only
  evidence_surface: page
  sensitive_exclusion: N/A
  submission_gate: approved
  story_id_or_title: System/SystemWorkspace/ModelsMobile
  state: light mobile
  evidence_note: verifies the responsive model list and row actions at the mobile viewport
  image:
  ![模型页手机浅色](./assets/models-page-mobile-light.png)

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  requested_viewport: 393x852
  viewport_strategy: storybook-viewport
  margin_policy: trim_only
  evidence_surface: page
  sensitive_exclusion: N/A
  submission_gate: approved
  story_id_or_title: System/SystemWorkspace/ModelsMobileDark
  state: dark mobile
  evidence_note: verifies the responsive model list in the dark theme
  image:
  ![模型页手机深色](./assets/models-page-mobile-dark.png)

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  requested_viewport: 1660x960
  viewport_strategy: storybook-viewport
  margin_policy: trim_only
  evidence_surface: page
  sensitive_exclusion: N/A
  submission_gate: approved
  story_id_or_title: System/SystemWorkspace/ModelsSyncReview
  state: desktop provider price review
  evidence_note: verifies provider search and groups, current-versus-source prices, conflict selection, default checkboxes, unsupported dimensions, and apply controls
  image:
  ![桌面同步审核弹窗](./assets/models-sync-review-desktop.png)

- source_type: storybook_canvas
  target_program: mock-only
  capture_scope: element
  requested_viewport: 393x852
  viewport_strategy: storybook-viewport
  margin_policy: trim_only
  evidence_surface: page
  sensitive_exclusion: N/A
  submission_gate: approved
  story_id_or_title: System/SystemWorkspace/ModelsSyncReviewMobile
  state: mobile provider price review
  evidence_note: verifies the responsive review dialog, independently scrollable candidates, provider selection, and fixed apply footer
  image:
  ![手机同步审核弹窗](./assets/models-sync-review-mobile.png)

## References

- [models.dev API documentation](https://github.com/anomalyco/models.dev/blob/dev/README.md#api)
- `./IMPLEMENTATION.md`
- `./HISTORY.md`
