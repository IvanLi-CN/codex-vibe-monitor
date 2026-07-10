# GPT-5.6 系列定价、缓存写入计费与模型入口支持

Spec ID: 97dds

## Background

The repo-managed pricing catalog, `/v1/models` preset list, and Settings pricing contract currently stop at the GPT-5.5 generation and only model one cached-input price. GPT-5.6 introduces three first-class model ids (`gpt-5.6-sol`, `gpt-5.6-terra`, `gpt-5.6-luna`) plus an explicit cache write price that is distinct from both uncached input and cache read pricing.

The project needs a compatible upgrade that preserves existing user-defined pricing rows and existing API consumers while making GPT-5.6 cost estimation, Settings editing, and operator-facing model selection accurate.

## Goals

- Add first-class support for `gpt-5.6-sol`, `gpt-5.6-terra`, and `gpt-5.6-luna` across the default pricing catalog, proxy preset models, Settings model lists, and `/v1/models` hijack payloads.
- Upgrade the pricing contract to support explicit cache read and cache write unit prices with a compatibility bridge for legacy `cacheInputPer1m`.
- Make `estimate_proxy_cost` bill GPT-5.6 cached tokens with read pricing and uncached prompt tokens with write pricing while keeping legacy-model semantics unchanged.
- Replace the remaining `unsupported_model:gpt-5.5` UI special-case with generic `unsupported_model:<model>` handling so newer unsupported models behave correctly without new hardcoding.

## Non-goals

- Do not add online pricing sync or import the full `sub2api` pricing payload.
- Do not invent a generic `gpt-5.6` placeholder model id.
- Do not expose derived `cacheWriteTokens` as if it were an upstream usage truth field in records, dashboard, or timeseries pages.
- Do not change legacy model pricing rules except where schema plumbing is required for backward compatibility.

## Requirements

- The repo-managed catalog version must advance to `openai-standard-2026-07-10`.
- The repo-managed catalog must contain:
  - `gpt-5.6-sol`: input `5.0`, output `30.0`, cache read `0.5`, cache write `6.25`
  - `gpt-5.6-terra`: input `2.5`, output `15.0`, cache read `0.25`, cache write `3.125`
  - `gpt-5.6-luna`: input `1.0`, output `6.0`, cache read `0.10`, cache write `1.25`
- `PUT /api/settings/pricing` must accept both legacy `cacheInputPer1m` and the new `cacheReadPer1m` / `cacheWritePer1m` fields.
- `GET /api/settings/pricing` must return the new fields and continue mirroring `cacheInputPer1m` from `cacheReadPer1m` during the compatibility window.
- SQLite persistence must preserve existing pricing rows and backfill read pricing from legacy data without overwriting user-defined values.
- Model resolution must match exact ids first and also map `gpt-5.6-sol|terra|luna-YYYY-MM-DD` to their base model pricing rows.
- Settings pricing UI must split cached pricing into separate cache read and cache write columns and clearly label the contract as estimation metadata rather than runtime token truth.

## Interface Contract

### Pricing entry shape

The backend and frontend pricing entry contract supports these unit-price fields in USD per one million tokens:

- `inputPer1m`
- `outputPer1m`
- `cacheReadPer1m`
- `cacheWritePer1m`
- `reasoningPer1m`

Legacy `cacheInputPer1m` remains an accepted write alias and a read mirror for `cacheReadPer1m`.

### Storage

`pricing_settings_models` includes both the legacy compatibility column and the new explicit cache columns:

- `cache_input_per_1m REAL NULL`
- `cache_read_per_1m REAL NULL`
- `cache_write_per_1m REAL NULL`

Rows that only have legacy cached-input pricing treat `cache_input_per_1m` as the cache read price.

### Cost estimation

- For entries with explicit `cacheReadPer1m` and `cacheWritePer1m`:
  - `cached_tokens` bill at `cacheReadPer1m`
  - `input_tokens - cached_tokens` bill at `cacheWritePer1m`
- For entries without explicit cache write pricing:
  - keep the existing behavior where uncached input bills at `inputPer1m`
  - cached input bills at the legacy cache read price when present

## Acceptance Criteria

- Given a legacy pricing payload with only `cacheInputPer1m`, when the backend saves and reloads it, then `cacheReadPer1m` matches that value and `cacheInputPer1m` is still mirrored on response.
- Given an existing SQLite database with legacy pricing rows, when the schema upgrade runs, then read pricing is preserved and no existing user-defined row is overwritten.
- Given `model=gpt-5.6-sol`, `input_tokens=1000`, `cached_tokens=400`, and `output_tokens=200`, when cost is estimated, then 600 prompt tokens bill at `6.25 / 1M`, 400 cached tokens bill at `0.5 / 1M`, and 200 output tokens bill at `30 / 1M`.
- Given `gpt-5.6-sol-2026-07-08`, `gpt-5.6-terra-2026-07-08`, or `gpt-5.6-luna-2026-07-08`, when cost is estimated, then the base GPT-5.6 pricing row is used rather than `unknown`.
- Given a legacy model entry that only has cached-input pricing, when cost is estimated, then existing legacy tests continue to use the pre-upgrade uncached-input semantics.
- Given default proxy model settings, when repo-managed defaults are normalized, then the GPT-5.6 model ids appear in preset lists and are appended only for legacy default enabled-model lists.
- Given account tags containing `unsupported_model:gpt-5.6-sol`, when the roster and routing UI render, then the tag behaves like other system unsupported-model tags without GPT-5.5-specific special casing.

## Visual Evidence

PR: include
![Settings pricing cache read/write split](./assets/settings-pricing-cache-read-write-storybook.png)

- source_type: storybook_canvas
- target_program: mock-only
- capture_scope: element
- requested_viewport: desktop1660
- viewport_strategy: storybook-viewport
- sensitive_exclusion: N/A
- submission_gate: pending-owner-approval
- story_id_or_title: Settings/SettingsPage Default
- state: default pricing contract editor
- evidence_note: Verifies the Settings pricing table exposes separate cache read and cache write columns, includes the GPT-5.6 trio, and labels the table as estimation contract metadata rather than runtime token truth.

## References

- OpenAI pricing announcement and API pricing pages published on 2026-07-08.
- `docs/archive/specs/7272y-gpt-5-4-pricing/SPEC.md`
- `docs/archive/specs/47ran-pool-models-override-gpt55-pricing/SPEC.md`
