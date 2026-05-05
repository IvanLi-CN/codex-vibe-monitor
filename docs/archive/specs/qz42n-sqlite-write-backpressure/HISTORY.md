# SQLite 写入可靠性与后台背压历史

## 2026-04-26

- 101 生产排查确认 15:30-16:30 CST 新增 OAuth 上游账号期间，应用返回 44 次 `502 Bad Gateway`，并出现 11 次 SQLite `database is locked`。
- 事故窗口内新增账号集中在 `2784-2791`，锁压力主要集中于 16:09-16:14 CST，日志出现 28-30 秒连接等待和 pool acquire timeout。
- 结论锁定为应用内 SQLite 写锁竞争、后台任务抢占与热点查询共同放大，而非 Traefik 网关自身 502。
- 本轮实现应用层背压：后台 DB 任务在 pressure 下 skip/backoff，前台关键路径保留连接池预算；同时补齐账号维护与 latest sample 热点索引。
