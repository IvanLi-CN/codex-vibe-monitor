# Implementation

- Adds `pool_tags.system_key` and `pool_tags.protected` schema maintenance.
- Ensures the protected system tag `unsupported_model:gpt-5.5` / `不支持 gpt-5.5` exists at startup.
- Records the system tag on the failing account when the unsupported `gpt-5.5` error is observed.
- Skips tagged accounts when selecting pool accounts for `gpt-5.5` requests.
- Extends tag API payloads with `systemKey` and `protected` fields.
- Blocks editing or deleting protected tags while leaving account-tag unlinking available.
- Adds Storybook coverage for the magenta unsupported-model badge state in the upstream accounts table.
