# 数据库（DB）

## External API keys table

- 范围（Scope）: internal
- 变更（Change）: New
- 影响表（Affected tables）: `external_api_keys`

### Schema delta（结构变更）

- DDL / migration snippet（尽量精简）:
  - `CREATE TABLE IF NOT EXISTS external_api_keys (...)`
  - 列：`id`、`client_id`、`name`、`secret_hash`、`secret_prefix`、`status`、`last_used_at`、`created_at`、`updated_at`、`rotated_from_key_id`
- Constraints / indexes:
  - `UNIQUE(secret_hash)`
  - `INDEX(client_id, status)`
  - `INDEX(rotated_from_key_id)`

### Migration notes（迁移说明）

- 向后兼容窗口（Backward compatibility window）: 新表增量创建，无旧数据迁移。
- 发布/上线步骤（Rollout steps）: 启动时自动建表；Settings 页面读写新接口。
- 回滚策略（Rollback strategy）: 停用新路由与 UI，保留表不影响旧功能。
- 回填/数据迁移（Backfill / data migration, 如适用）: none

## External upstream mapping columns

- 范围（Scope）: internal
- 变更（Change）: Modify
- 影响表（Affected tables）: `pool_upstream_accounts`

### Schema delta（结构变更）

- DDL / migration snippet（尽量精简）:
  - `ALTER TABLE pool_upstream_accounts ADD COLUMN external_client_id TEXT`
  - `ALTER TABLE pool_upstream_accounts ADD COLUMN external_source_account_id TEXT`
- Constraints / indexes:
  - `UNIQUE INDEX (external_client_id, external_source_account_id) WHERE both NOT NULL`

### Migration notes（迁移说明）

- 向后兼容窗口（Backward compatibility window）: 历史账号保留 `NULL` 映射，不影响内部入口。
- 发布/上线步骤（Rollout steps）: 先部署 schema，再开放 external API。
- 回滚策略（Rollback strategy）: 停用 external API；映射列保留但不会影响旧查询。
- 回填/数据迁移（Backfill / data migration, 如适用）: none
