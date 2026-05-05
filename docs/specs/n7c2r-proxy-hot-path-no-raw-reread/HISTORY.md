# 代理热路径停止 response raw 二次回读 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/n7c2r-proxy-hot-path-no-raw-reread/SPEC.md`

## Migrated History Notes

## Change log

- 将 proxy capture 成功热路径中的 raw-file SSE hint / response parse fallback 从在线请求链路移除，保留 raw helper 作为非热路径能力，并补上“热路径 raw reread 为零”的回归断言。
