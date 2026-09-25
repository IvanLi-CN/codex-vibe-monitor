# Dashboard 对外调用时间线

> This file is the durable topic requirements contract. Current implementation facts belong in `IMPLEMENTATION.md`; lifecycle and change references belong in `HISTORY.md`.

## Context and Scope

- Context: Dashboard 的自然日次数视图需要同时表达调用发生时间、并行关系、总耗时、状态和 TTFT。
- In scope: 今日、昨日及账号范围的自然日调用时间线、时间窗口接口、实时更新与明细上限回退。
- Out of scope: 金额、Tokens、网速、24 小时、7 日、历史聚合图表，以及调用计时和数据保留策略。

## Terms and Interfaces

- `Invocation`: 一次对外调用，以 `invokeId` 和 `occurredAt` 标识；同一次调用的上游重试不产生新的时间线横条。
- `Virtual lane`: 按开始时间分配给调用的最低空闲轨道，用来表达并发关系。
- Interface: `GET /api/stats/invocation-timeline` accepts UTC `from`, `to`, and optional `upstreamAccountId`.

## Requirements

### REQ-DIT-001

- The system MUST render one horizontal bar per external invocation, with the bar starting at `occurredAt` and ending at the valid terminal `tTotalMs` endpoint.
- Inputs: terminal records and live runtime records in a requested UTC window.
- Outputs: retries remain folded into the same `invokeId` bar; missing or invalid terminal duration is shown as unknown rather than an invented endpoint.

### REQ-DIT-002

- The system MUST assign each invocation to the lowest virtual lane that is idle at its start time and MUST expose parallel, running, and queued counts at the hovered time.

### REQ-DIT-003

- The system MUST show valid per-invocation TTFT markers and the existing minute-level average TTFT curve in a separate aligned region sharing the invocation time axis.

### REQ-DIT-004

- The system MUST provide a bounded overlap query for global and account scopes, return `asOf` and `overLimit`, and return no partial detail when more than 2,000 invocations overlap the requested window.

### REQ-DIT-005

- The system MUST update today's live bars from the authoritative activity revision or a refresh after reconnect, pause local live extension while disconnected, and use HTTP-only data for yesterday.

### REQ-DIT-006

- The system MUST default natural-day views to a 30-minute detail window, support zoom and pan within the full-day overview, and fall back visibly to the existing aggregate chart when offline or over the detail limit.

## Verification

### VER-DIT-001

- Method: Rust timeline overlap tests and the timeline endpoint contract test fixture.
- covers: `REQ-DIT-001`, `REQ-DIT-004`
- Pass condition: cross-midnight records overlap correctly, retries are represented once, and over-limit responses do not contain incomplete records.

### VER-DIT-002

- Method: frontend lane and API normalization unit tests.
- covers: `REQ-DIT-002`, `REQ-DIT-003`
- Pass condition: lowest idle lane, zero-duration visibility, in-flight extension, account filtering, and valid TTFT-only rendering remain stable.

### VER-DIT-003

- Method: responsive Storybook and dashboard render evidence.
- covers: `REQ-DIT-005`, `REQ-DIT-006`
- Pass condition: desktop and mobile views remain readable, zoom/pan controls work, and offline or over-limit states show the aggregate fallback.

## Related ADRs

None

## Visual Evidence

- None

## References

- `./IMPLEMENTATION.md`
- `./HISTORY.md`
