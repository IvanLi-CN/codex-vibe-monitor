# Encrypted Session Owner Guard Implementation

- Adds `prompt_cache_encrypted_session_owners` schema and owner lookup/upsert helpers.
- Extends request/response capture with `contains_encrypted_content`.
- Persists encrypted owner metadata into Prompt Cache conversation APIs and binding APIs.
- Locks successful encrypted conversations to the current upstream account and promotes group overrides to account bindings after success.
- Returns a dedicated owner-unavailable failure when automatic selection cannot keep the encrypted session on its owner.
- Adds Rust regression coverage for owner metadata visibility, clearing manual bindings without clearing owner lock, group-to-account promotion, and the dedicated owner-unavailable failover guard.
- Updates Storybook/demo fixtures and optimistic live prompt-cache mocks so the new encrypted-owner fields remain type-safe under `bun run build` and `bun run test`.
- Adds Storybook owner-lock coverage for the Prompt Cache conversation drawer and persists a mock visual-evidence capture in the spec assets.
- Uses the shared project `Dialog` component for dangerous owner-rebinding confirmation in the Prompt Cache conversation drawer, with Storybook coverage that fails if native `window.confirm` is used.
