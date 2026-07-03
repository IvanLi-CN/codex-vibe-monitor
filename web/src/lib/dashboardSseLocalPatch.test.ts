import { describe, expect, it } from "vitest";
import type {
  ApiInvocation,
  StatsResponse,
  UpstreamAccountActivityResponse,
} from "./api";
import {
  createDashboardRecordPatchState,
  filterDashboardRecordsForLocalDay,
  patchDashboardSummaryWithRecords,
  patchUpstreamAccountActivityWithRecords,
  seedDashboardSummaryPatchState,
  seedUpstreamAccountActivityPatchState,
} from "./dashboardSseLocalPatch";
import { normalizeEffectiveRoutingRule } from "./api/core-upstream";

function record(overrides: Partial<ApiInvocation> & { id: number; invokeId: string; status: string }): ApiInvocation {
  return {
    id: overrides.id,
    invokeId: overrides.invokeId,
    occurredAt: overrides.occurredAt ?? "2026-07-03T10:00:00.000Z",
    createdAt: overrides.createdAt ?? overrides.occurredAt ?? "2026-07-03T10:00:00.000Z",
    status: overrides.status,
    totalTokens: overrides.totalTokens ?? 0,
    cost: overrides.cost ?? 0,
    model: overrides.model ?? "gpt-5.5",
    endpoint: overrides.endpoint ?? "/v1/responses",
    routeMode: overrides.routeMode ?? "pool",
    upstreamAccountId: overrides.upstreamAccountId ?? null,
    upstreamAccountName: overrides.upstreamAccountName,
    failureClass: overrides.failureClass,
    failureKind: overrides.failureKind,
    errorMessage: overrides.errorMessage,
    downstreamErrorMessage: overrides.downstreamErrorMessage,
    isActionable: overrides.isActionable,
    poolAttemptCount: overrides.poolAttemptCount,
  };
}

function summary(overrides: Partial<StatsResponse> = {}): StatsResponse {
  return {
    totalCount: 10,
    successCount: 8,
    failureCount: 2,
    totalCost: 1,
    totalTokens: 1000,
    nonSuccessCost: 0.2,
    nonSuccessTokens: 120,
    inProgressConversationCount: 3,
    inProgressRetryConversationCount: 1,
    ...overrides,
  };
}

function accountResponse(): UpstreamAccountActivityResponse {
  return {
    range: "today",
    rangeStart: "2026-07-03T00:00:00.000Z",
    rangeEnd: "2026-07-03T10:05:00.000Z",
    accounts: [
      {
        upstreamAccountId: 42,
        displayName: "Pool Alpha",
        groupName: "Primary",
        planType: "enterprise",
        requestCount: 4,
        successCount: 3,
        failureCount: 1,
        nonSuccessCount: 1,
        totalTokens: 400,
        successTokens: 300,
        nonSuccessTokens: 100,
        failureTokens: 100,
        failureCost: 0.1,
        totalCost: 0.4,
        cacheHitRate: 0.2,
        tokensPerMinute: 90,
        spendRate: 0.08,
        firstByteAvgMs: 220,
        firstResponseByteTotalAvgMs: 260,
        avgTotalMs: 900,
        inProgressInvocationCount: 1,
        retryInvocationCount: 0,
        effectiveRoutingRule: normalizeEffectiveRoutingRule({}),
        recentInvocations: [],
      },
    ],
  };
}

describe("dashboard SSE local patch helpers", () => {
  it("patches terminal summary counters without counting in-flight records as completed", () => {
    const patchState = createDashboardRecordPatchState();
    const patched = patchDashboardSummaryWithRecords(summary(), [
      record({ id: 11, invokeId: "success-1", status: "success", totalTokens: 90, cost: 0.09 }),
      record({ id: 12, invokeId: "failed-1", status: "failed", totalTokens: 30, cost: 0.03 }),
      record({ id: 13, invokeId: "running-1", status: "running", poolAttemptCount: 2 }),
    ], patchState);

    expect(patched).toMatchObject({
      totalCount: 12,
      successCount: 9,
      failureCount: 3,
      totalCost: 1.12,
      totalTokens: 1120,
      nonSuccessCost: 0.23,
      nonSuccessTokens: 150,
      inProgressConversationCount: 4,
      inProgressRetryConversationCount: 2,
    });
  });

  it("classifies success-like records with failure metadata like server aggregates", () => {
    const patchState = createDashboardRecordPatchState();
    const patched = patchDashboardSummaryWithRecords(summary(), [
      record({
        id: 14,
        invokeId: "success-with-failure-class",
        status: "success",
        failureClass: "service_failure",
        totalTokens: 40,
        cost: 0.04,
      }),
      record({
        id: 15,
        invokeId: "http-200-with-error",
        status: "http_200",
        errorMessage: "upstream response stream reported failure",
        totalTokens: 30,
        cost: 0.03,
      }),
    ], patchState);

    expect(patched).toMatchObject({
      totalCount: 12,
      successCount: 8,
      failureCount: 4,
      totalCost: 1.07,
      totalTokens: 1070,
      nonSuccessCost: 0.27,
      nonSuccessTokens: 190,
    });
  });

  it("filters local patches to the active local day", () => {
    const records = [
      record({ id: 1, invokeId: "today", status: "success", occurredAt: "2026-07-03T10:00:00.000Z" }),
      record({ id: 2, invokeId: "yesterday", status: "success", occurredAt: "2026-07-02T15:59:59.000Z" }),
      record({ id: 3, invokeId: "tomorrow", status: "success", occurredAt: "2026-07-04T00:00:00.000Z" }),
    ];

    expect(
      filterDashboardRecordsForLocalDay(
        records,
        new Date("2026-07-03T12:00:00.000Z").getTime(),
      ).map((item) => item.invokeId),
    ).toEqual(["today"]);
  });

  it("dedupes repeated records and replaces running contribution with terminal contribution", () => {
    const patchState = createDashboardRecordPatchState();
    const first = record({ id: 1, invokeId: "same", status: "running", poolAttemptCount: 2 });
    const duplicate = record({ id: 1, invokeId: "same", status: "running", poolAttemptCount: 2 });
    const terminalUpdate = record({ id: 1, invokeId: "same", status: "success", totalTokens: 10, cost: 0.01 });

    const afterRunning = patchDashboardSummaryWithRecords(summary(), [first], patchState);
    expect(afterRunning).toMatchObject({
      inProgressConversationCount: 4,
      inProgressRetryConversationCount: 2,
      totalCount: 10,
    });

    expect(patchDashboardSummaryWithRecords(afterRunning, [duplicate], patchState)).toBe(afterRunning);

    const afterTerminal = patchDashboardSummaryWithRecords(afterRunning, [terminalUpdate], patchState);
    expect(afterTerminal).toMatchObject({
      inProgressConversationCount: 3,
      inProgressRetryConversationCount: 1,
      totalCount: 11,
      successCount: 9,
      totalTokens: 1010,
    });
  });

  it("updates retry counters when a running record gains retry metadata", () => {
    const patchState = createDashboardRecordPatchState();
    const running = record({ id: 2, invokeId: "retrying", status: "running", poolAttemptCount: 1 });
    const retrying = record({ id: 2, invokeId: "retrying", status: "running", poolAttemptCount: 2 });

    const afterRunning = patchDashboardSummaryWithRecords(summary(), [running], patchState);
    expect(afterRunning).toMatchObject({
      inProgressConversationCount: 4,
      inProgressRetryConversationCount: 1,
    });

    const afterRetrying = patchDashboardSummaryWithRecords(afterRunning, [retrying], patchState);
    expect(afterRetrying).toMatchObject({
      inProgressConversationCount: 4,
      inProgressRetryConversationCount: 2,
    });
  });

  it("seeds hydrated summary in-flight budget before terminal records arrive", () => {
    const patchState = createDashboardRecordPatchState();
    seedDashboardSummaryPatchState(
      patchState,
      summary({
        inProgressConversationCount: 1,
        inProgressRetryConversationCount: 1,
      }),
      Date.parse("2026-07-03T10:00:00.000Z"),
    );

    const patched = patchDashboardSummaryWithRecords(summary({
      inProgressConversationCount: 1,
      inProgressRetryConversationCount: 1,
    }), [
      record({
        id: 31,
        invokeId: "hydrated-running",
        status: "success",
        createdAt: "2026-07-03T09:59:59.000Z",
        totalTokens: 20,
        cost: 0.02,
        poolAttemptCount: 2,
      }),
    ], patchState);

    expect(patched).toMatchObject({
      totalCount: 11,
      successCount: 9,
      inProgressConversationCount: 0,
      inProgressRetryConversationCount: 0,
    });
  });

  it("absorbs already hydrated running summary records before terminal updates", () => {
    const patchState = createDashboardRecordPatchState();
    seedDashboardSummaryPatchState(
      patchState,
      summary({
        inProgressConversationCount: 1,
        inProgressRetryConversationCount: 1,
      }),
      Date.parse("2026-07-03T10:00:00.000Z"),
    );
    const current = summary({
      inProgressConversationCount: 1,
      inProgressRetryConversationCount: 1,
    });

    const running = patchDashboardSummaryWithRecords(current, [
      record({
        id: 36,
        invokeId: "hydrated-running-update",
        status: "running",
        createdAt: "2026-07-03T09:59:59.000Z",
        poolAttemptCount: 2,
      }),
    ], patchState);

    expect(running).toBe(current);

    const terminal = patchDashboardSummaryWithRecords(running, [
      record({
        id: 36,
        invokeId: "hydrated-running-update",
        status: "success",
        createdAt: "2026-07-03T09:59:59.000Z",
        totalTokens: 20,
        cost: 0.02,
        poolAttemptCount: 2,
      }),
    ], patchState);

    expect(terminal).toMatchObject({
      totalCount: 11,
      successCount: 9,
      inProgressConversationCount: 0,
      inProgressRetryConversationCount: 0,
    });
  });

  it("does not treat pre-hydration starts as covered when hydrated in-flight budget remains", () => {
    const patchState = createDashboardRecordPatchState();
    seedDashboardSummaryPatchState(
      patchState,
      summary({
        inProgressConversationCount: 1,
        inProgressRetryConversationCount: 0,
      }),
      Date.parse("2026-07-03T10:00:00.000Z"),
    );

    const patched = patchDashboardSummaryWithRecords(summary({
      inProgressConversationCount: 1,
      inProgressRetryConversationCount: 0,
    }), [
      record({
        id: 35,
        invokeId: "long-running-before-hydrate",
        status: "success",
        createdAt: "2026-07-03T09:59:00.000Z",
        totalTokens: 20,
        cost: 0.02,
      }),
    ], patchState);

    expect(patched).toMatchObject({
      totalCount: 11,
      successCount: 9,
      inProgressConversationCount: 0,
    });
  });

  it("does not double-count terminal summary records already covered by hydration", () => {
    const patchState = createDashboardRecordPatchState();
    seedDashboardSummaryPatchState(
      patchState,
      summary({ inProgressConversationCount: 0, inProgressRetryConversationCount: 0 }),
      Date.parse("2026-07-03T10:00:00.000Z"),
    );

    const coveredRecord = record({
      id: 41,
      invokeId: "already-hydrated-terminal",
      status: "success",
      createdAt: "2026-07-03T09:59:59.000Z",
      totalTokens: 20,
      cost: 0.02,
    });
    const current = summary({
      inProgressConversationCount: 0,
      inProgressRetryConversationCount: 0,
    });

    const afterCovered = patchDashboardSummaryWithRecords(current, [coveredRecord], patchState);
    expect(afterCovered).toBe(current);

    const afterDuplicate = patchDashboardSummaryWithRecords(current, [coveredRecord], patchState);
    expect(afterDuplicate).toBe(current);
  });

  it("patches only existing upstream account cards and reports missed accounts for reconcile", () => {
    const patched = patchUpstreamAccountActivityWithRecords(
      accountResponse(),
      [
        record({
          id: 21,
          invokeId: "alpha-success",
          status: "success",
          upstreamAccountId: 42,
          upstreamAccountName: "Pool Alpha",
          totalTokens: 70,
          cost: 0.07,
        }),
        record({
          id: 22,
          invokeId: "new-account",
          status: "success",
          upstreamAccountId: 99,
          upstreamAccountName: "Pool New",
          totalTokens: 40,
          cost: 0.04,
        }),
      ],
      4,
      createDashboardRecordPatchState(),
    );

    expect(patched.missedAccountRecord).toBe(true);
    expect(patched.response?.accounts).toHaveLength(1);
    expect(patched.response?.accounts[0]).toMatchObject({
      requestCount: 5,
      successCount: 4,
      totalTokens: 470,
      totalCost: 0.47000000000000003,
    });
    expect(patched.response?.accounts[0]?.recentInvocations[0]?.invokeId).toBe("alpha-success");
  });

  it("counts running upstream account rows like the server activity aggregate", () => {
    const patched = patchUpstreamAccountActivityWithRecords(
      accountResponse(),
      [
        record({
          id: 24,
          invokeId: "alpha-running",
          status: "running",
          upstreamAccountId: 42,
          upstreamAccountName: "Pool Alpha",
          totalTokens: 70,
          cost: 0.07,
        }),
      ],
      4,
      createDashboardRecordPatchState(),
    );

    expect(patched.response?.accounts[0]).toMatchObject({
      requestCount: 5,
      successCount: 3,
      failureCount: 1,
      totalTokens: 470,
      successTokens: 300,
      totalCost: 0.47000000000000003,
      inProgressInvocationCount: 2,
    });
  });

  it("classifies upstream account success-like records with failure metadata like server aggregates", () => {
    const patched = patchUpstreamAccountActivityWithRecords(
      accountResponse(),
      [
        record({
          id: 25,
          invokeId: "account-success-with-failure-kind",
          status: "completed",
          upstreamAccountId: 42,
          upstreamAccountName: "Pool Alpha",
          failureKind: "upstream_stream_error",
          totalTokens: 80,
          cost: 0.08,
        }),
      ],
      4,
      createDashboardRecordPatchState(),
    );

    expect(patched.response?.accounts[0]).toMatchObject({
      requestCount: 5,
      successCount: 3,
      failureCount: 2,
      nonSuccessCount: 2,
      totalTokens: 480,
      successTokens: 300,
      failureTokens: 180,
      nonSuccessTokens: 180,
      failureCost: 0.18,
      totalCost: 0.48000000000000004,
    });
  });

  it("patches today account records that arrive after the stale HTTP range end", () => {
    const current = accountResponse();
    const patchState = createDashboardRecordPatchState();
    seedUpstreamAccountActivityPatchState(
      patchState,
      current,
      Date.parse("2026-07-03T10:05:00.000Z"),
    );

    const patched = patchUpstreamAccountActivityWithRecords(
      current,
      [
        record({
          id: 27,
          invokeId: "today-after-hydration-range-end",
          status: "success",
          occurredAt: "2026-07-03T10:05:01.000Z",
          createdAt: "2026-07-03T10:05:01.000Z",
          upstreamAccountId: 42,
          upstreamAccountName: "Pool Alpha",
          totalTokens: 50,
          cost: 0.05,
        }),
      ],
      4,
      patchState,
    );

    expect(patched.response?.accounts[0]).toMatchObject({
      requestCount: 5,
      successCount: 4,
      totalTokens: 450,
      totalCost: 0.45,
    });
  });

  it("does not consume hydrated account in-flight budget for records created after hydration", () => {
    const current = accountResponse();
    current.accounts[0]!.inProgressInvocationCount = 1;
    const patchState = createDashboardRecordPatchState();
    seedUpstreamAccountActivityPatchState(
      patchState,
      current,
      Date.parse("2026-07-03T10:00:00.000Z"),
    );

    const patched = patchUpstreamAccountActivityWithRecords(
      current,
      [
        record({
          id: 25,
          invokeId: "new-after-hydrate-terminal",
          status: "success",
          createdAt: "2026-07-03T10:00:01.000Z",
          upstreamAccountId: 42,
          upstreamAccountName: "Pool Alpha",
          totalTokens: 50,
          cost: 0.05,
        }),
      ],
      4,
      patchState,
    );

    expect(patched.response?.accounts[0]).toMatchObject({
      requestCount: 5,
      successCount: 4,
      inProgressInvocationCount: 1,
    });
  });

  it("absorbs already hydrated unknown running account records before terminal updates", () => {
    const current = accountResponse();
    current.accounts[0]!.inProgressInvocationCount = 1;
    current.accounts[0]!.retryInvocationCount = 1;
    current.accounts[0]!.recentInvocations = [];
    const patchState = createDashboardRecordPatchState();
    seedUpstreamAccountActivityPatchState(
      patchState,
      current,
      Date.parse("2026-07-03T10:00:00.000Z"),
    );

    const running = patchUpstreamAccountActivityWithRecords(
      current,
      [
        record({
          id: 33,
          invokeId: "hydrated-unknown-account-running",
          status: "running",
          createdAt: "2026-07-03T09:59:59.000Z",
          upstreamAccountId: 42,
          upstreamAccountName: "Pool Alpha",
          totalTokens: 50,
          cost: 0.05,
          poolAttemptCount: 2,
        }),
      ],
      4,
      patchState,
    );

    expect(running.response?.accounts[0]).toMatchObject({
      requestCount: 4,
      totalTokens: 400,
      totalCost: 0.4,
      inProgressInvocationCount: 1,
      retryInvocationCount: 1,
    });

    const terminal = patchUpstreamAccountActivityWithRecords(
      running.response,
      [
        record({
          id: 33,
          invokeId: "hydrated-unknown-account-running",
          status: "success",
          createdAt: "2026-07-03T09:59:59.000Z",
          upstreamAccountId: 42,
          upstreamAccountName: "Pool Alpha",
          totalTokens: 50,
          cost: 0.05,
          poolAttemptCount: 2,
        }),
      ],
      4,
      patchState,
    );

    expect(terminal.response?.accounts[0]).toMatchObject({
      requestCount: 4,
      successCount: 4,
      totalTokens: 400,
      successTokens: 350,
      totalCost: 0.4,
      inProgressInvocationCount: 0,
      retryInvocationCount: 0,
    });
  });

  it("does not consume hydrated summary in-flight budget for records created after hydration", () => {
    const patchState = createDashboardRecordPatchState();
    seedDashboardSummaryPatchState(
      patchState,
      summary({
        inProgressConversationCount: 1,
        inProgressRetryConversationCount: 0,
      }),
      Date.parse("2026-07-03T10:00:00.000Z"),
    );

    const patched = patchDashboardSummaryWithRecords(summary({
      inProgressConversationCount: 1,
      inProgressRetryConversationCount: 0,
    }), [
      record({
        id: 26,
        invokeId: "new-summary-after-hydrate-terminal",
        status: "success",
        createdAt: "2026-07-03T10:00:01.000Z",
        totalTokens: 20,
        cost: 0.02,
      }),
    ], patchState);

    expect(patched).toMatchObject({
      totalCount: 11,
      successCount: 9,
      inProgressConversationCount: 1,
    });
  });

  it("seeds hydrated account in-flight rows before terminal records arrive", () => {
    const current = accountResponse();
    current.accounts[0]!.recentInvocations = [
      {
        ...record({
          id: 32,
          invokeId: "hydrated-account-running",
          status: "running",
          upstreamAccountId: 42,
          upstreamAccountName: "Pool Alpha",
          poolAttemptCount: 2,
        }),
        failureClass: "none",
        inputTokens: null,
        outputTokens: null,
        cacheInputTokens: null,
        reasoningTokens: null,
        reasoningEffort: null,
        proxyDisplayName: null,
        tReqReadMs: null,
        tReqParseMs: null,
        tUpstreamConnectMs: null,
        tUpstreamTtfbMs: null,
        tUpstreamStreamMs: null,
        tRespParseMs: null,
        tPersistMs: null,
        tTotalMs: null,
      },
    ];
    const patchState = createDashboardRecordPatchState();
    seedUpstreamAccountActivityPatchState(patchState, current);

    const patched = patchUpstreamAccountActivityWithRecords(
      current,
      [
        record({
          id: 32,
          invokeId: "hydrated-account-running",
          status: "success",
          upstreamAccountId: 42,
          upstreamAccountName: "Pool Alpha",
          totalTokens: 50,
          cost: 0.05,
          poolAttemptCount: 2,
        }),
      ],
      4,
      patchState,
    );

    expect(patched.response?.accounts[0]).toMatchObject({
      requestCount: 4,
      successCount: 4,
      inProgressInvocationCount: 0,
      retryInvocationCount: 0,
    });
  });

  it("does not double count hidden hydrated account in-flight requests when terminal SSE arrives", () => {
    const current = accountResponse();
    current.accounts[0]!.inProgressInvocationCount = 1;
    current.accounts[0]!.retryInvocationCount = 1;
    current.accounts[0]!.recentInvocations = [];
    const patchState = createDashboardRecordPatchState();
    seedUpstreamAccountActivityPatchState(
      patchState,
      current,
      Date.parse("2026-07-03T10:05:00.000Z"),
    );

    const patched = patchUpstreamAccountActivityWithRecords(
      current,
      [
        record({
          id: 39,
          invokeId: "hydrated-hidden-account-terminal",
          status: "success",
          upstreamAccountId: 42,
          upstreamAccountName: "Pool Alpha",
          createdAt: "2026-07-03T09:59:59.000Z",
          totalTokens: 50,
          cost: 0.05,
          poolAttemptCount: 2,
        }),
      ],
      4,
      patchState,
    );

    expect(patched.response?.accounts[0]).toMatchObject({
      requestCount: 4,
      successCount: 4,
      totalTokens: 450,
      totalCost: 0.45,
      inProgressInvocationCount: 0,
      retryInvocationCount: 0,
    });
  });

  it("seeds hydrated account terminal rows so late SSE duplicates do not inflate totals", () => {
    const current = accountResponse();
    current.accounts[0]!.recentInvocations = [
      {
        ...record({
          id: 43,
          invokeId: "hydrated-account-terminal",
          status: "success",
          upstreamAccountId: 42,
          upstreamAccountName: "Pool Alpha",
          totalTokens: 70,
          cost: 0.07,
        }),
        failureClass: "none",
        inputTokens: null,
        outputTokens: null,
        cacheInputTokens: null,
        reasoningTokens: null,
        reasoningEffort: null,
        proxyDisplayName: null,
        tReqReadMs: null,
        tReqParseMs: null,
        tUpstreamConnectMs: null,
        tUpstreamTtfbMs: null,
        tUpstreamStreamMs: null,
        tRespParseMs: null,
        tPersistMs: null,
        tTotalMs: null,
      },
    ];
    const patchState = createDashboardRecordPatchState();
    seedUpstreamAccountActivityPatchState(patchState, current);

    const patched = patchUpstreamAccountActivityWithRecords(
      current,
      [
        record({
          id: 43,
          invokeId: "hydrated-account-terminal",
          status: "success",
          upstreamAccountId: 42,
          upstreamAccountName: "Pool Alpha",
          totalTokens: 70,
          cost: 0.07,
        }),
      ],
      4,
      patchState,
    );

    expect(patched.response).toBe(current);
    expect(patched.response?.accounts[0]).toMatchObject({
      requestCount: 4,
      successCount: 3,
      totalTokens: 400,
      totalCost: 0.4,
    });
  });
});
