# Rust 后端测试运行时反馈环

## 适用场景

- Rust 后端测试数量很大，`cargo test` 输出被编译和 warning 噪音淹没，难以判断真实测试运行时。
- 测试大量创建 SQLite 临时库、archive gzip 文件或完整应用状态，默认并发下出现 60s runner warning。
- 慢点主要来自 fixture 搭建，而不是生产逻辑本身。

## 核心结论

- 先用 `cargo test --no-run` 生成热缓存 test binary，再直接运行 `target/debug/deps/<bin> <filter> --format=terse`，用 `/usr/bin/time -p` 测单测和全套运行时。
- CI 总时长优化要以 `CI Main` 里 3 个 backend required jobs 的最慢 wall time 为主指标：`Backend Tests (Lightweight)`、`Backend Tests (Stateful SQLite)`、`Backend Tests (Archive / File I/O)`。
- backend runner 应固定用 resource-profile filter 跑 `cargo nextest run --locked --all-features --no-fail-fast -E ...`，不要再把整个后端测试树塞回单个 required check。
- 对只验证 DB 行为、不验证主库文件路径的测试，优先使用唯一命名的 in-memory SQLite，保留 `AppConfig.database_path`、archive/raw 目录形状即可。
- 文件 SQLite fixture 要限制测试连接池大小；默认 pool 在全套并发下会把每个测试放大成多连接竞争。
- 如果测试只需要“已 materialized archive metadata”或“缺失 replay marker”状态，直接构造窄表状态，不要为了 setup 跑完整 retention/archive pipeline。
- 对确实验证 archive 文件内容或文件主库行为的测试，保留文件 SQLite，并把它们作为剩余 top offenders 明确列出。

## 推荐反馈环

```sh
cargo test --no-run
bin=$(find target/debug/deps -maxdepth 1 -type f -perm -111 -name 'codex_vibe_monitor-*' | head -1)
/usr/bin/time -p "$bin" archive_backfill_respects_scan_limit_budget --format=terse
/usr/bin/time -p "$bin" --test-threads=4 --format=terse
bash .github/scripts/run-backend-tests.sh --profile lightweight
bash .github/scripts/run-backend-tests.sh --profile stateful-sqlite
bash .github/scripts/run-backend-tests.sh --profile archive-file-io
```

## 常见坑

- 不要把首轮编译时间当成慢测试时间；先分离 compile 和 hot test execution。
- 不要只报告局部单测变快；PR 要同时报告 split 后各 GitHub backend job 总时长、各 profile runner wall time，以及 top offenders 变化。
- 切换到 nextest 前先修掉并发暴露的测试竞态；真实时间窗口断言要以行为结果为主，毫秒上限只作为防挂死保护。
- 不要为了速度把所有文件 SQLite 测试切成 in-memory；archive writer/reader、relative path、真实 write-lock 行为需要文件 DB。
- 单跑很快但全套出现 60s warning，通常是并发资源放大；先看 `sys` time 和 SQLite pool 数量，再决定是否下沉 fixture。
- 直接构造窄状态时要保留被测语义，例如 materialized archive 需要 `historical_rollups_materialized_at` 和必要 replay marker 状态，否则测试会误触发 archive 文件读取。
