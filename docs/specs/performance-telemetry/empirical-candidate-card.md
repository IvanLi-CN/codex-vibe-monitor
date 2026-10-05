# 性能遥测经验性验收卡

这份卡片是已退役 SQLite 性能系统的历史证据，仅适用于下述旧候选版本，不能用于
外部观测系统的验收或 Ready 判断。当前合同见 [SPEC](SPEC.md) 与
[部署验收入口](../../../ops/observability/README.md)；新工具使用 `--candidate`、
`--samply`、`--seconds` 和 `--rate`，不再接受下文的旧场景参数。

## 候选镜像

当前候选使用 Linux 镜像
`codex_codex-vibe-monitor__715fe933_20260927_200129_performance_b9f76d87_26484`。
试验在 `codex-testbox` 的 Agent Directory 下运行，关闭和启用组使用同一镜像、同一负载入口。当前注册表为 104 条近期序列和 22 条长期序列，共 126 条固定序列。

本次运行目录：

- 启用组：`/srv/codex/agents/01a0de1f-0b81-7253-b9c8-58b13776b239/runs/20260927_200129_performance_b9f76d87_26484`
- 关闭组：`/srv/codex/agents/01a0de1f-0b81-7253-b9c8-58b13776b239/runs/20260927_202726_performance_b9f76d87_63687`
- 容量组：`/srv/codex/agents/01a0de1f-0b81-7253-b9c8-58b13776b239/runs/20260927_210820_performance_b9f76d87_48544`
- A6 低负载禁用诊断：`/srv/codex/agents/01a0de1f-0b81-7253-b9c8-58b13776b239/runs/20260927_210211_performance_b9f76d87_26497`
- A6 低负载启用诊断：`/srv/codex/agents/01a0de1f-0b81-7253-b9c8-58b13776b239/runs/20260927_210520_performance_b9f76d87_38223`
- A6 50 ms mock 禁用复跑：`/srv/codex/agents/01a0de1f-0b81-7253-b9c8-58b13776b239/runs/20260927_213612_performance_b9f76d87_13691`
- A6 50 ms mock 启用复跑：`/srv/codex/agents/01a0de1f-0b81-7253-b9c8-58b13776b239/runs/20260927_215536_performance_b9f76d87_56316`
- 正式 overhead 禁用组：`/srv/codex/agents/01a0de1f-0b81-7253-b9c8-58b13776b239/runs/20260928_003940_performance_b9f76d87_39710`
- 正式 overhead 启用组：`/srv/codex/agents/01a0de1f-0b81-7253-b9c8-58b13776b239/runs/20260928_005644_performance_b9f76d87_72808`
- 正式 overhead 禁用组（P1 控制）：`/srv/codex/agents/01a0de1f-0b81-7253-b9c8-58b13776b239/runs/20260928_031533_performance_b9f76d87_44433`
- 正式 overhead 启用组（P1 控制）：`/srv/codex/agents/01a0de1f-0b81-7253-b9c8-58b13776b239/runs/20260928_033228_performance_b9f76d87_91389`
- overhead 第三次配对（关闭组无效）：`/srv/codex/agents/01a0de1f-0b81-7253-b9c8-58b13776b239/runs/20260928_040014_performance_b9f76d87_63244`
- overhead 第四次配对（关闭组无效）：`/srv/codex/agents/01a0de1f-0b81-7253-b9c8-58b13776b239/runs/20260928_042354_performance_b9f76d87_17615`
- 低速率维护压力诊断关闭组：`/srv/codex/agents/01a0de1f-0b81-7253-b9c8-58b13776b239/runs/20260928_105548_performance_b9f76d87_43841`
- 低速率维护压力诊断启用组：`/srv/codex/agents/01a0de1f-0b81-7253-b9c8-58b13776b239/runs/20260928_110548_performance_b9f76d87_43840`
- 预热后维护压力诊断（关闭遥测）：`/srv/codex/agents/01a0de1f-0b81-7253-b9c8-58b13776b239/runs/20260928_112442_performance_b9f76d87_13389`

## A6 代表性负载诊断（非合并门禁）

A6 仅作为发布后诊断项保留。合并前不把共享测试机上的代理/P1 ACK p95 增量、CPU 或 RSS 差异解释为遥测开销，除非关闭与启用组都在同一稳定窗口完成完整对照。A7 容量、写锁隔离、故障恢复和直接质量门禁仍是交付要求。

当前候选 `0d9cf196` 在共享机恢复后的禁用遥测复跑目录为：
`/srv/codex/agents/01a0de1f-0b81-7253-b9c8-58b13776b239/runs/20260928_125033_performance_0d9cf196_295`。
前两轮分别完成 3,000/3,000 请求，P95 为 91.20 ms 和 184.94 ms，P1 ACK P95 为 6 ms 和 8 ms；第三轮出现 13 个 30 秒超时，P95 为 9,897.63 ms。遥测关闭组没有丢弃，健康接口为 `disabled`，但该基线不满足稳定对照条件，因此没有启动对应的启用组，也不改变 A6 的诊断项结论。

## 50 req/s 业务容量压力（非 A6）

入口为 `scripts/shared-testbox-performance-acceptance --scenario sustained --duration 300 --rounds 3 --rate 50`。
每轮包含 50 req/s 代理请求、16 条 dashboard SSE、请求完成驱动的 P1 终态写入；每组连续三轮，每轮 5 分钟。

### 关闭采集

| 轮次 |     提交/完成 |  失败 | 队列丢弃 |          P50 |          P95 |       最大值 |   SSE |
| ---- | ------------: | ----: | -------: | -----------: | -----------: | -----------: | ----: |
| 1    | 4,402 / 4,402 |   106 |        2 |  5,218.22 ms | 23,660.89 ms | 30,239.77 ms | 16/16 |
| 2    | 1,257 / 1,257 | 1,257 |      266 | 30,033.18 ms | 30,092.62 ms | 30,330.93 ms | 16/16 |
| 3    | 1,256 / 1,256 | 1,256 |      274 | 30,031.38 ms | 30,042.92 ms | 30,290.81 ms | 16/16 |

容器 Docker CPU 平均 114.52%、P95 239.16%、峰值 321.98%；内存快照最高 381.5 MiB。遥测查询按预期返回 503，独立健康接口返回 `disabled`。

### 启用采集

| 轮次 |     提交/完成 | 失败 | 队列丢弃 |         P50 |          P95 |       最大值 |   SSE |
| ---- | ------------: | ---: | -------: | ----------: | -----------: | -----------: | ----: |
| 1    | 6,191 / 6,191 |    0 |        0 | 4,076.63 ms | 12,387.21 ms | 29,696.92 ms | 16/16 |
| 2    | 4,244 / 4,244 |   11 |        5 | 6,318.13 ms | 16,845.65 ms | 30,032.78 ms | 16/16 |
| 3    | 3,652 / 3,652 |   16 |        0 | 7,846.47 ms | 19,578.79 ms | 30,036.29 ms | 16/16 |

容器 Docker CPU 平均 342.59%、P95 522.61%、峰值 558.20%；内存快照最高 724.6 MiB。遥测健康最终为 `healthy`，`droppedSamples=0`、`flushFailureCount=0`。

### 压力结论

这组压力数据未达到 50 req/s 的业务容量目标，但结果不能单独归因于遥测。两组都出现 30 秒超时和吞吐下降，关闭组从第一轮开始就无法维持 50 req/s；启用组的 CPU 平均值高 228.07 个 Docker 百分点、P95 高 283.45 个百分点，内存最高快照高 343.1 MiB。由于关闭组已经饱和，两个指标的绝对差值不是有效的遥测增量估计；它暴露了当前代理/存储路径在 16 条 SSE 并发下的容量风险。该场景不再作为 A6 的通过/失败依据。

### 低负载诊断

这组数据用于为 `overhead` 场景选择速率。相同镜像下以 10 req/s、16 条 SSE、单轮 60 秒运行：关闭组和启用组均为 600/600 成功、0 队列丢弃、0 SSE 错误；关闭组 P95 为 1,216.42 ms、CPU 平均 17.54%，启用组 P95 为 1,215.23 ms、CPU 平均 17.49%，遥测健康为 `healthy`，`droppedSamples=0`。因此当前候选选择 10 req/s 作为未饱和 A6 速率。

### 正式 A6 首次完整对照

两组使用同一候选镜像、10 req/s、50 ms mock 上游、16 条 SSE、三轮五分钟；每组 9,000 个请求全部完成，0 队列丢弃、0 SSE 错误，16 条 SSE 全部连接。

| 组别     |    R1 P95 |      R2 P95 |      R3 P95 |       CPU 平均 / P95 / 峰值 |  RSS 最高 |
| -------- | --------: | ----------: | ----------: | --------------------------: | --------: |
| 关闭采集 | 327.27 ms | 1,281.44 ms | 1,261.96 ms | 143.69% / 218.55% / 234.13% | 420.1 MiB |
| 启用采集 | 505.98 ms |   497.32 ms | 1,118.57 ms | 138.48% / 208.01% / 217.82% | 424.2 MiB |

资源结果为 CPU 平均下降 3.63 个百分点、RSS 增加 4.1 MiB，满足 CPU 及 RSS 阈值；三轮代理 P95 受运行顺序影响，不能用单轮最大差值宣称固定增量。启用组遥测健康为 `healthy`，查询 200，`droppedSamples=0`、`flushFailureCount=0`；关闭组查询 503、健康接口为 `disabled`。

启用组独立库中的 P1 ACK 采样为 92 个样本，直方图 P95 落在 `<=50 ms` 桶；关闭组尚未同时保存 P1 ACK 控制样本。因此 A6 的 P1 ACK 对照仍未完成，当前候选不标记 A6 通过。

后续带 System Status 采样的复跑未纳入证据：每秒采样会改变负载，降至 10 秒后测试机又受到其他 Agent 的 `rustc`/`sha256sum` 占用并出现超时，不能作为未饱和对照。

### 正式 A6 第二次完整对照（含 P1 控制）

这次关闭组和启用组使用同一候选镜像、同一 `overhead` 负载和同一 System Status 采样方式：10 req/s、50 ms mock 上游、16 条 SSE、三轮五分钟，P1 ACK 每 10 秒采样一次。两组均完成 9,000/9,000 个请求，0 队列丢弃、0 SSE 错误；每轮取得 31 个 P1 ACK 样本，状态接口错误为 0。

| 组别     |    R1 P95 |    R2 P95 |    R3 P95 | P1 ACK R1/R2/R3 P95 |       CPU 平均 / P95 / 峰值 |  RSS 最高 |
| -------- | --------: | --------: | --------: | ------------------: | --------------------------: | --------: |
| 关闭采集 |  81.63 ms | 100.58 ms | 135.89 ms |       4 / 6 / 10 ms | 175.37% / 235.83% / 247.02% | 342.0 MiB |
| 启用采集 | 106.47 ms | 167.09 ms | 148.77 ms |       5 / 9 / 28 ms | 161.73% / 233.81% / 242.97% | 289.7 MiB |

启用组 CPU 平均值低 13.64 个百分点、最高 RSS 低 52.3 MiB，遥测查询 58.61 ms、126 条序列、覆盖率 `0.001388888888888889`，健康状态为 `healthy`，`droppedSamples=0`、`flushFailureCount=0`；关闭组查询 503、健康状态为 `disabled`。这些结果没有显示遥测写入风暴或资源增量。

按 A6 的逐轮门槛计算，代理 P95 增量分别为 +24.84 ms、+66.51 ms、+12.88 ms，P1 ACK P95 增量分别为 +1 ms、+3 ms、+18 ms；代理三轮和 P1 第三轮均超过 `max(5%, 5 ms)`。因此本轮 A6 不通过。由于两组的延迟随运行顺序明显漂移，且资源指标反而下降，本轮只能证明采集库健康和业务成功，不能把延迟差异全部归因于遥测；需要在更稳定的共享机窗口再次复测后才能决定是否达到 A6。

第三、四次配对没有纳入证据：第三次关闭组第二轮在共享机竞争下出现 4 个队列丢弃、105 个 30 秒超时，P95 为 19,216.75 ms；第四次关闭组第二轮出现 25 个队列丢弃、170 个 30 秒超时，P95 为 30,029.55 ms。两次都未达到未饱和对照条件，均在启动启用组前停止并清理了本次运行创建的容器。连续两次在第一轮成功后于第二轮失去稳定性，说明当前共享机/持续负载组合本身不适合作为新的 A6 增量证据；不再重复同一场景。

### 低速率维护压力诊断（非 A6）

为区分主库后台维护压力和遥测写入开销，另以 1 req/s、50 ms mock 上游、16 条 SSE、120 秒三轮运行，并保留每 10 秒一次的 P1 ACK 采样。关闭组和启用组均为 360/360 请求成功、0 队列丢弃、0 SSE 错误。

| 组别     |               R1/R2/R3 P95 | P1 ACK R1/R2/R3 P95 | CPU 平均 / 峰值 | 遥测状态 |
| -------- | -------------------------: | ------------------: | --------------: | -------- |
| 关闭采集 |   78.72 / 71.14 / 81.86 ms |        3 / 3 / 4 ms |   3.27% / 6.61% | disabled |
| 启用采集 | 372.44 / 335.22 / 75.46 ms |      13 / 76 / 5 ms |   3.10% / 7.53% | healthy  |

启用组查询耗时 95.21 ms，`droppedSamples=0`、`flushFailureCount=0`，独立库没有写入失败或队列积压。但启用组前两轮出现明显延迟和 P1 ACK 峰值，应用日志同时记录 `sqlite_batch_writer_p1` 的 `sqlite_busy`、P1 flush 退避；关闭组也记录后台 `system_task_runs` 写入 1.8–9.3 秒的慢语句。两组都受到主库 Summary coverage recovery、系统维护和 SQLite 写入协调竞争影响，所以这组只证明低速率下业务仍可完成，并不能单独归因于遥测或标记 A6 通过。

### 预热后维护压力诊断（非 A6）

为验证“启动后多等一段时间即可消除干扰”的可能性，关闭遥测组使用同一候选镜像、10 req/s、50 ms mock 上游、16 条 SSE，先等待 120 秒再测量两轮各 60 秒。两轮均完成 600/600 请求、0 队列丢弃、0 SSE 错误，代理 P95 为 66.93 / 69.52 ms，P1 ACK P95 为 3 / 3 ms；遥测查询按关闭组契约返回 503，健康状态为 `disabled`。

预热期间应用日志仍从启动后第一个 10 秒周期开始持续记录 `summary historical coverage recovery deferred`，同时记录 long-term materialization 的 `background_busy`。因此静置不能形成没有主库维护争用的测量窗口；继续重复同一正式 A6 负载不会增加归因证据。该诊断只用于确认阻塞条件，不改变 A6 未通过结论。

### A6 50 ms mock 复跑

为排除原始 800/1,200 ms mock 上游延迟，验收脚本新增 `--mock-delay-ms 50`，保持 50 req/s、16 条 SSE 和三轮 5 分钟不变。关闭组 P95 为 14,473.05 / 22,294.22 / 23,280.62 ms，启用组为 23,486.22 / 30,030.96 / 26,651.36 ms；关闭组 CPU 平均 333.32%、最高 RSS 622.9 MiB，启用组 CPU 平均 242.76%、最高 RSS 564.4 MiB。两组仍然出现超时和排队，启用组没有 CPU/RSS 增量，但 P95 差异受主库容量和运行顺序影响；这组结果只保留为容量诊断，不再重复同一饱和场景。

## A7 容量与退避

入口为 `scripts/shared-testbox-performance-acceptance --scenario capacity`，使用当前候选镜像注入 43,200 个分钟桶和 16 个固定序列，再等待 rollup、执行查询和 75 秒 SQLite 写锁故障注入。

| 项目                      | 实测结果                                                                                     | 判定                                                           |
| ------------------------- | -------------------------------------------------------------------------------------------- | -------------------------------------------------------------- |
| 30 天查询（当前注册表）   | 复跑 1,138.01 ms 首次、1,154.96 ms 写锁恢复后；126 条固定序列元数据；覆盖率 1.0；最多 720 点 | 通过；此前同镜像一次恢复查询为 3,200.76 ms，作为偶发慢样本保留 |
| 写锁期间业务健康          | 75 秒探针内 `/health` 失败 0 次                                                              | 通过                                                           |
| 采集失败与恢复            | 释放后 `state=healthy`、`flushFailureCount=2`、`droppedSamples=0`、队列深度 0                | 通过                                                           |
| checkpoint 后 WAL         | 0 bytes                                                                                      | 通过                                                           |
| 遥测库文件                | 126,566,400 bytes（约 120.7 MiB）；SHM 229,376 bytes；WAL 0 bytes                            | 通过                                                           |
| 13 个月估算（当前注册表） | 22 条长期序列 × 9,360 小时桶约 205,920 行；按约 487 bytes/行估算约 96 MiB                    | 通过                                                           |

## 可复现实验

正式 A6 使用 `overhead` 场景；默认 10 req/s、50 ms mock 上游、16 条 SSE、三轮五分钟。50 req/s 的 `sustained` 命令仅用于业务容量压力，不能用于判断遥测增量。

```bash
PERFORMANCE_TELEMETRY_ENABLED=false \
  CODEX_THREAD_ID=<agent-directory> \
  scripts/shared-testbox-performance-acceptance \
  --testbox codex-testbox \
  --reuse-image codex_codex-vibe-monitor__715fe933_20260927_200129_performance_b9f76d87_26484 \
  --scenario overhead --capture-p1-ack

PERFORMANCE_TELEMETRY_ENABLED=true \
  CODEX_THREAD_ID=<agent-directory> \
  scripts/shared-testbox-performance-acceptance \
  --testbox codex-testbox \
  --reuse-image codex_codex-vibe-monitor__715fe933_20260927_200129_performance_b9f76d87_26484 \
  --scenario overhead --capture-p1-ack

# 低速率维护压力诊断，仅用于区分主库维护竞争，不作为 A6 门禁
PERFORMANCE_TELEMETRY_ENABLED=false \
  CODEX_THREAD_ID=<agent-directory> \
  scripts/shared-testbox-performance-acceptance \
  --testbox codex-testbox \
  --reuse-image codex_codex-vibe-monitor__715fe933_20260927_200129_performance_b9f76d87_26484 \
  --scenario overhead --duration 120 --rounds 3 --rate 1 --capture-p1-ack

PERFORMANCE_TELEMETRY_ENABLED=true \
  CODEX_THREAD_ID=<agent-directory> \
  scripts/shared-testbox-performance-acceptance \
  --testbox codex-testbox \
  --reuse-image codex_codex-vibe-monitor__715fe933_20260927_200129_performance_b9f76d87_26484 \
  --scenario overhead --duration 120 --rounds 3 --rate 1 --capture-p1-ack

# 预热后维护诊断：验证启动静置是否能消除主库维护争用
PERFORMANCE_TELEMETRY_ENABLED=false \
  CODEX_THREAD_ID=<agent-directory> \
  scripts/shared-testbox-performance-acceptance \
  --testbox codex-testbox \
  --reuse-image codex_codex-vibe-monitor__715fe933_20260927_200129_performance_b9f76d87_26484 \
  --scenario overhead --warmup-seconds 120 --duration 60 --rounds 2 --rate 10 --capture-p1-ack
```

```bash
PERFORMANCE_TELEMETRY_ENABLED=false \
  CODEX_THREAD_ID=<agent-directory> \
  scripts/shared-testbox-performance-acceptance \
  --testbox codex-testbox \
  --reuse-image codex_codex-vibe-monitor__715fe933_20260927_200129_performance_b9f76d87_26484 \
  --scenario sustained --duration 300 --rounds 3 --rate 50 --mock-delay-ms 50

PERFORMANCE_TELEMETRY_ENABLED=true \
  CODEX_THREAD_ID=<agent-directory> \
  scripts/shared-testbox-performance-acceptance \
  --testbox codex-testbox \
  --reuse-image codex_codex-vibe-monitor__715fe933_20260927_200129_performance_b9f76d87_26484 \
  --scenario sustained --duration 300 --rounds 3 --rate 50 --mock-delay-ms 50

PERFORMANCE_TELEMETRY_ENABLED=true \
  CODEX_THREAD_ID=<agent-directory> \
  scripts/shared-testbox-performance-acceptance \
  --testbox codex-testbox \
  --reuse-image codex_codex-vibe-monitor__715fe933_20260927_200129_performance_b9f76d87_26484 \
  --scenario capacity
```
