# raw 负载冷压缩与磁盘全文搜索 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/jg7a5-raw-payload-cold-compression-search/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-13: 创建 spec，锁定 `gzip + 24h 热明文 + 单文件 gzip + docker exec search-raw` 作为本轮唯一方案。
- 2026-03-13: 已完成后端冷压缩、透明解压、retention 统计与搜索脚本实现，等待文档与验证收口。
- 2026-03-13: 已完成 README / deployment / spec 索引同步，`cargo fmt --check`、`cargo check`、`cargo test` 全部通过；本地 Docker daemon 不可用，镜像 smoke 待有 daemon 环境时补跑。
- 2026-03-13: 根据 PR 阶段 review 修复 cold-compress 分页游标与 `search-raw` gzip 无命中退出码，并补齐对应回归测试。
- 2026-03-13: 调整 `search-raw` 默认 root 解析，使其跟随 `DATABASE_PATH + PROXY_RAW_DIR`，并把缺失 root 改为显式配置错误退出码。
- 2026-03-13: 收紧透明解压到真实 `.gz` 路径，避免误判普通二进制 raw；`search-raw` 对损坏 gzip 改为显式报错退出，避免假阴性。
- 2026-03-13: 修复冷压缩 repair 分支对相对 raw path 的绝对路径回写，并将 `search-raw` 的 gzip 搜索改为流式解压，避免额外临时明文文件依赖。
- 2026-03-13: 冷压缩阶段跳过已进入 prune/archive 窗口的记录，避免在磁盘吃紧时先复制再清理；单文件压缩失败改为告警并继续后续 retention。
