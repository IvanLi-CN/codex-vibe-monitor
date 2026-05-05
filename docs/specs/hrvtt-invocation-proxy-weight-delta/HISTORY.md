# 请求详情补齐代理信息与本次权重变化 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/hrvtt-invocation-proxy-weight-delta/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.

## Migrated Change History

## 变更记录（Change log）

- 2026-03-02: 初始化规格，冻结“最简代理信息 + 仅Δ + 历史不回填”口径。
- 2026-03-02: 完成后端字段投影与详情展示改造，新增回归测试并通过 `cargo` 与 `web` 质量检查。
- 2026-03-02: 根据产品反馈将详情展示调整为“彩色箭头 + 无符号两位小数”，并补充可访问性文案要求。
