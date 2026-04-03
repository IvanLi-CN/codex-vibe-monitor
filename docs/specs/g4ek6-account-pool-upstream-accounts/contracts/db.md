# DB Contract

## `pool_upstream_accounts`

- `id INTEGER PRIMARY KEY AUTOINCREMENT`
- `kind TEXT NOT NULL` (`oauth_codex | api_key_codex`)
- `provider TEXT NOT NULL DEFAULT 'codex'`
- `display_name TEXT NOT NULL`
- `note TEXT NULL`
- `status TEXT NOT NULL` (`active | syncing | needs_reauth | error`)
- `enabled INTEGER NOT NULL DEFAULT 1`
- `email TEXT NULL`
- `chatgpt_account_id TEXT NULL`
- `chatgpt_user_id TEXT NULL`
- `plan_type TEXT NULL`
- `masked_api_key TEXT NULL`
- `encrypted_credentials TEXT NULL`
- `token_expires_at TEXT NULL`
- `last_refreshed_at TEXT NULL`
- `last_synced_at TEXT NULL`
- `last_successful_sync_at TEXT NULL`
- `last_error TEXT NULL`
- `last_error_at TEXT NULL`
- `local_primary_limit REAL NULL`
- `local_secondary_limit REAL NULL`
- `local_limit_unit TEXT NULL`
- `created_at TEXT NOT NULL`
- `updated_at TEXT NOT NULL`

索引：

- `idx_pool_upstream_accounts_kind_enabled`
- `idx_pool_upstream_accounts_chatgpt_account_id`

## `pool_oauth_login_sessions`

- `login_id TEXT PRIMARY KEY`
- `account_id INTEGER NULL`
- `display_name TEXT NULL`
- `group_name TEXT NULL`
- `group_bound_proxy_keys_json TEXT NOT NULL DEFAULT '[]'`
- `group_node_shunt_enabled INTEGER NOT NULL DEFAULT 0`
- `note TEXT NULL`
- `state TEXT NOT NULL UNIQUE`
- `pkce_verifier TEXT NOT NULL`
- `redirect_uri TEXT NOT NULL`
- `status TEXT NOT NULL` (`pending | completed | failed | expired`)
- `auth_url TEXT NOT NULL`
- `error_message TEXT NULL`
- `expires_at TEXT NOT NULL`
- `consumed_at TEXT NULL`
- `created_at TEXT NOT NULL`
- `updated_at TEXT NOT NULL`

## `pool_upstream_account_group_notes`

- `group_name TEXT PRIMARY KEY`
- `note TEXT NULL`
- `bound_proxy_keys_json TEXT NOT NULL DEFAULT '[]'`
- `node_shunt_enabled INTEGER NOT NULL DEFAULT 0`
- `upstream_429_retry_enabled INTEGER NOT NULL DEFAULT 0`
- `upstream_429_max_retries INTEGER NOT NULL DEFAULT 0`
- `updated_at TEXT NOT NULL`

## `pool_upstream_account_limit_samples`

- `id INTEGER PRIMARY KEY AUTOINCREMENT`
- `account_id INTEGER NOT NULL`
- `captured_at TEXT NOT NULL`
- `limit_id TEXT NULL`
- `limit_name TEXT NULL`
- `plan_type TEXT NULL`
- `primary_used_percent REAL NULL`
- `primary_window_minutes INTEGER NULL`
- `primary_resets_at TEXT NULL`
- `secondary_used_percent REAL NULL`
- `secondary_window_minutes INTEGER NULL`
- `secondary_resets_at TEXT NULL`
- `credits_has_credits INTEGER NULL`
- `credits_unlimited INTEGER NULL`
- `credits_balance TEXT NULL`

索引：

- `idx_pool_limit_samples_account_captured_at`
