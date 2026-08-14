---
title: Rust backend test runtime feedback loop
module: backend-tests
problem_type: performance
component: Rust backend CI
tags:
  - rust
  - nextest
  - sqlite
  - ci
  - test-runtime
status: active
related_specs:
  - docs/specs/q7yt7-backend-test-resource-profiles/SPEC.md
---

# Rust 后端测试运行时反馈环

## 适用场景

- Rust 后端测试数量很大，`cargo test` 输出被编译和 warning 噪音淹没，难以判断真实测试运行时。
- 测试大量创建 SQLite 临时库、archive gzip 文件或完整应用状态，默认并发下出现 runner 慢点和 SQLite 资源竞争。
- CI wall time 同时包含冷编译与测试执行，单一总时长无法确定应该优化哪一层。

## 根因

- Stateful 端到端测试会继承生产保护性的 retry/backoff 与 no-available-account 等待；若 harness 没有显式区分行为验证和真实时间验证，睡眠会累计为 CI 关键路径。
- 大请求 file-backed 语义测试若用远高于分支阈值的 payload，会把内存分配、序列化与文件 I/O 成本误当成覆盖价值。
- schema 模板只有在 schema/default-data parity、pooled connection 可见性、并发写和跨测试隔离都成立时才会快；每个 nextest 子进程内再生成模板会重复 setup，必须由 runner 先生成一次，并用 SQLite backup API 复制到各自的 shared-memory 数据库。

## 核心结论

- 先分离冷编译、热 profile、单 fixture 初始化、真实等待和线程并发成本。热数据只能帮助定位测试执行，不能替代 CI critical path 验收。
- 当前基线是 `CI Main` run `31706131099`：Stateful SQLite wall time `617s`，compile `143s`，test execution `404s`；backend-related jobs 总计 `1257s`。
- CI 主指标是从 PR workflow start 到 `Backend Tests (Stateful SQLite)` completed。当前预算 `<= 390s`，且同一 PR head 必须连续两次通过。
- backend runner 应固定用 resource-profile filter 跑 `cargo nextest run --locked --all-features --no-fail-fast -E ...`，不要再把整个后端测试树塞回单个 required check。
- 当前 1213 个 Stateful SQLite 用例在 4、6、8 threads 各两次热运行均通过。4 threads 为 `134.108s` / `83.952s`，6 threads 为 `63.593s` / `62.874s`，8 threads 为 `57.287s` / `67.727s`；8 的平均值最快，但 6 在线 10% 内，因此 runner 选择 6。
- `run-backend-tests.sh --archive-file <path>` 可复用预先生成的 nextest archive，但 CI 保留它前必须同一 PR head 两次满足 Stateful `<= 390s`，且 backend runner 秒数 `<= 1005s`。单次 archive 命令或本地速度不是保留依据。
- 两个 CI archive 原型都说明“编译复用”不等于关键路径改善：独立 producer 的 run `31811122919` Stateful critical path 为 `504s`、backend runner 为 `872s`；Stateful job 内构建并分发的 run `31813566813` 为 `433s`、`750s`。两者都应从最终 workflow 移除，并仅保留 runner 的可选 archive 参数供后续受控实验。
- 对只验证 DB 行为、不验证主库文件路径的测试，使用唯一命名的 in-memory SQLite；legacy migration、文件路径、gzip 和 write-lock 保留真实 schema/file fixture。
- runner 先由一次真实 `ensure_schema` 生成私有 file template；每个唯一 shared-memory SQLite 再通过 SQLite backup API 获得副本。必须验证 schema/default-data parity、pooled connection visibility、双向写入与跨测试隔离；shared-memory serialize/deserialize、逐条 SQL dump 和直接文件副本都不是最终路径。
- current-schema-only 的服务层级回填、成本回填、内存启动错误分类、定价重载和默认 source-scope 测试应优先复用 template pool；不得把 legacy migration、文件路径、gzip 或 write-lock 测试迁入该路径。
- 如果测试只需要“已 materialized archive metadata”或“缺失 replay marker”状态，直接构造窄表状态，不要为了 setup 跑完整 retention/archive pipeline。
- 对确实验证 archive 文件内容或文件主库行为的测试，保留文件 SQLite，并把它们作为剩余 top offenders 明确列出。

## 推荐反馈环

```sh
cargo nextest run --locked --all-features --no-fail-fast --test-threads=4 -E 'test(/^(tests|upstream_accounts::tests)::stateful_sqlite::/)'
cargo nextest run --locked --all-features --no-fail-fast --test-threads=6 -E 'test(/^(tests|upstream_accounts::tests)::stateful_sqlite::/)'
cargo nextest run --locked --all-features --no-fail-fast --test-threads=8 -E 'test(/^(tests|upstream_accounts::tests)::stateful_sqlite::/)'
bash .github/scripts/run-backend-tests.sh --profile lightweight
bash .github/scripts/run-backend-tests.sh --profile stateful-sqlite
bash .github/scripts/run-backend-tests.sh --profile archive-file-io
cargo nextest archive --locked --all-features --archive-file /tmp/backend-tests.tar.zst
bash .github/scripts/run-backend-tests.sh --archive-file /tmp/backend-tests.tar.zst
```

## 常见坑

- 不要把首轮编译时间当成慢测试时间；冷编译、热执行、fixture、真实 delay 与线程并发必须分别记录。
- 不要只报告局部单测变快；PR 要同时报告两次 workflow-start Stateful wall time、三个 profile 完整通过、backend runner 总秒数与 top offenders。
- 切换到 nextest 前先修掉并发暴露的测试竞态；真实时间窗口断言要以行为结果为主，毫秒上限只作为防挂死保护。
- 对 retry/backoff 与 no-available-account 轮询，测试 harness 可注入零等待，但 production wrapper/default 与需要验证时间预算的测试必须保留正式值。
- 大请求的 file-backed 语义可由私有 memory threshold 注入较小输入；仍要保留一条正式阈值边界测试，不能把生产 threshold 改成测试值。
- 不要为了速度把所有文件 SQLite 测试切成 in-memory；archive writer/reader、relative path、真实 write-lock 行为需要文件 DB。
- 先验证 4、6、8 等候选并发档位的完整 profile，各至少两次；选择最快档位 10% 内的最低档位。
- 单跑很快但全套出现 60s warning，通常是并发资源放大；先看 `sys` time 和 SQLite pool 数量，再决定是否下沉 fixture。
- 直接构造窄状态时要保留被测语义，例如 materialized archive 需要 `historical_rollups_materialized_at` 和必要 replay marker 状态，否则测试会误触发 archive 文件读取。

## Top-offender 排查顺序

1. 先排除测试默认 retry/backoff 与 no-available-account wait，确认慢点不是为生产保护而设计的真实 sleep。
2. 再检查大请求是否为 threshold 分支分配了不必要的多十 MiB payload，缩小测试输入但保留正式阈值边界覆盖。
3. 然后检查 State fixture、schema 初始化和 SQLite pool contention；只有完整隔离证据成立才引入 schema 模板。
4. 最后保留并标记真正的 write-lock、retention、archive/gzip、文件路径慢例；这些是行为覆盖，不是可删除的“膨胀”。
