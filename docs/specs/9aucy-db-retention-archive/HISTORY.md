# 数据分层保留、离线归档与长周期汇总 - History

## Migration

- Canonical docs taxonomy migration created or normalized this companion history file.
- Canonical spec: `docs/specs/9aucy-db-retention-archive/SPEC.md`

## Key Decisions

- No separate historical decision record was present before this migration.
- 2026-06-18: 收口 summary / timeseries 的 mixed archive/live 读路径，明确 `previous7d` 这类自然日 summary 必须复用 hourly rollup + full-hour live tail replay + uncovered archive fallback，不能只靠当前 retention cutoff 决定是否 live-only。
