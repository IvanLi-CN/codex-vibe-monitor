/** @vitest-environment jsdom */
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type {
  ApiInvocation,
  BroadcastPayload,
  PromptCacheConversationInvocationPreview,
  UpstreamAccountActivityResponse,
} from "../lib/api";
import { DASHBOARD_SSE_VISIBLE_PATCH_BATCH_MS } from "../lib/dashboardSseLocalPatch";
import {
  DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MAX,
  DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MIN,
} from "./useDashboardWorkingConversations";
import {
  resolveUpstreamAccountRecentPreviewLimit,
  useDashboardUpstreamAccountActivity,
} from "./useDashboardUpstreamAccountActivity";

const apiMocks = vi.hoisted(() => ({
  fetchUpstreamAccountActivity:
    vi.fn<
      (
        range: string,
        options?: { recentLimit?: number; timeZone?: string; signal?: AbortSignal },
      ) => Promise<UpstreamAccountActivityResponse>
    >(),
}));

const sseMocks = vi.hoisted(() => ({
  listeners: new Set<(payload: BroadcastPayload) => void>(),
  openListeners: new Set<() => void>(),
}));

vi.mock("../lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchUpstreamAccountActivity: apiMocks.fetchUpstreamAccountActivity,
  };
});

vi.mock("../lib/sse", () => ({
  subscribeToSse: (listener: (payload: BroadcastPayload) => void) => {
    sseMocks.listeners.add(listener);
    return () => sseMocks.listeners.delete(listener);
  },
  subscribeToSseOpen: (listener: () => void) => {
    sseMocks.openListeners.add(listener);
    return () => sseMocks.openListeners.delete(listener);
  },
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  sseMocks.listeners.clear();
  sseMocks.openListeners.clear();
  vi.useRealTimers();
  vi.clearAllMocks();
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

async function flushAsync() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

function text(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`);
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing element: ${testId}`);
  }
  return element.textContent ?? "";
}

function createPreview(
  overrides: Partial<PromptCacheConversationInvocationPreview> & {
    id: number;
    invokeId: string;
    occurredAt: string;
    status: string;
  },
): PromptCacheConversationInvocationPreview {
  return {
    id: overrides.id,
    invokeId: overrides.invokeId,
    promptCacheKey:
      "promptCacheKey" in overrides ? (overrides.promptCacheKey ?? null) : null,
    occurredAt: overrides.occurredAt,
    status: overrides.status,
    failureClass: overrides.failureClass ?? "none",
    routeMode: overrides.routeMode ?? "pool",
    model: overrides.model ?? "gpt-5.4",
    requestModel:
      "requestModel" in overrides ? (overrides.requestModel ?? null) : "gpt-5.4",
    responseModel:
      "responseModel" in overrides
        ? (overrides.responseModel ?? null)
        : (overrides.model ?? "gpt-5.4"),
    totalTokens: overrides.totalTokens ?? 120,
    cost: overrides.cost ?? 0.01,
    proxyDisplayName:
      "proxyDisplayName" in overrides
        ? (overrides.proxyDisplayName ?? null)
        : "tokyo-edge-01",
    upstreamAccountId:
      "upstreamAccountId" in overrides
        ? (overrides.upstreamAccountId ?? null)
        : 42,
    upstreamAccountName:
      "upstreamAccountName" in overrides
        ? (overrides.upstreamAccountName ?? null)
        : "Pool Alpha",
    upstreamAccountPlanType:
      "upstreamAccountPlanType" in overrides
        ? (overrides.upstreamAccountPlanType ?? null)
        : null,
    endpoint: overrides.endpoint ?? "/v1/responses",
    compactionRequestKind: overrides.compactionRequestKind ?? null,
    compactionResponseKind: overrides.compactionResponseKind ?? null,
    imageIntent: overrides.imageIntent ?? null,
    inputTokens: overrides.inputTokens ?? 70,
    outputTokens: overrides.outputTokens ?? 50,
    cacheInputTokens: overrides.cacheInputTokens ?? 0,
    reasoningTokens: overrides.reasoningTokens ?? 0,
    reasoningEffort: overrides.reasoningEffort ?? "medium",
    errorMessage: overrides.errorMessage,
    downstreamStatusCode: overrides.downstreamStatusCode,
    downstreamErrorMessage: overrides.downstreamErrorMessage,
    failureKind: overrides.failureKind,
    transport: overrides.transport,
    requestedServiceTier: overrides.requestedServiceTier ?? "priority",
    serviceTier: overrides.serviceTier ?? "priority",
    tReqReadMs: overrides.tReqReadMs ?? 10,
    tReqParseMs: overrides.tReqParseMs ?? 8,
    tUpstreamConnectMs: overrides.tUpstreamConnectMs ?? 40,
    tUpstreamTtfbMs: overrides.tUpstreamTtfbMs ?? 35,
    tUpstreamStreamMs: overrides.tUpstreamStreamMs ?? 90,
    tRespParseMs: overrides.tRespParseMs ?? 6,
    tPersistMs: overrides.tPersistMs ?? 4,
    tTotalMs: overrides.tTotalMs ?? 180,
  };
}

function createRecord(
  overrides: Partial<ApiInvocation> & {
    id: number;
    invokeId: string;
    occurredAt?: string;
    status: string;
  },
): ApiInvocation {
  return {
    id: overrides.id,
    invokeId: overrides.invokeId,
    occurredAt: overrides.occurredAt ?? "2026-04-04T10:04:30Z",
    createdAt: overrides.createdAt ?? overrides.occurredAt ?? "2026-04-04T10:04:30Z",
    status: overrides.status,
    source: overrides.source ?? "pool",
    routeMode: overrides.routeMode ?? "pool",
    model: overrides.model ?? "gpt-5.5",
    endpoint: overrides.endpoint ?? "/v1/responses",
    totalTokens: overrides.totalTokens ?? 0,
    cost: overrides.cost ?? 0,
    upstreamAccountId: overrides.upstreamAccountId ?? 42,
    upstreamAccountName: overrides.upstreamAccountName ?? "Pool Alpha",
    poolAttemptCount: overrides.poolAttemptCount,
  };
}

function emitRecords(records: ApiInvocation[]) {
  act(() => {
    sseMocks.listeners.forEach((listener) => {
      listener({ type: "records", records });
    });
  });
}

function createAccountResponse(
  inProgressInvocationCount: number,
  recentInvocations: PromptCacheConversationInvocationPreview[],
): UpstreamAccountActivityResponse {
  return {
    range: "today",
    rangeStart: "2026-04-04T10:00:00Z",
    rangeEnd: "2026-04-04T10:05:00Z",
    accounts: [
      {
        upstreamAccountId: 42,
        displayName: "Pool Alpha",
        groupName: "Primary",
        planType: "enterprise",
        requestCount: recentInvocations.length,
        successCount: recentInvocations.filter((item) => item.status === "success")
          .length,
        failureCount: recentInvocations.filter((item) => item.status === "failed")
          .length,
        nonSuccessCount: recentInvocations.filter(
          (item) => item.status === "failed",
        ).length,
        totalTokens: recentInvocations.reduce(
          (sum, item) => sum + item.totalTokens,
          0,
        ),
        successTokens: 480,
        nonSuccessTokens: 120,
        failureTokens: 120,
        failureCost: 0.04,
        totalCost: 0.12,
        cacheHitRate: 0.15,
        tokensPerMinute: 250,
        spendRate: 0.03,
        firstByteAvgMs: 320,
        firstResponseByteTotalAvgMs: 780,
        avgTotalMs: 780,
        inProgressInvocationCount,
        retryInvocationCount: 1,
        recentInvocations,
      },
    ],
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function Probe({
  enabled = true,
  range = "today",
  recentInvocationLimit = DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MIN,
}: {
  enabled?: boolean;
  range?: string;
  recentInvocationLimit?: number;
}) {
  const { data, isLoading, error, recentInvocationLimit: visibleLimit } =
    useDashboardUpstreamAccountActivity(range, enabled, recentInvocationLimit);

  return (
    <div>
      <div data-testid="loading">{isLoading ? "true" : "false"}</div>
      <div data-testid="error">{error ?? ""}</div>
      <div data-testid="visible-limit">{String(visibleLimit)}</div>
      <div data-testid="recent-count">
        {String(data?.accounts[0]?.recentInvocations.length ?? 0)}
      </div>
      <div data-testid="request-count">
        {String(data?.accounts[0]?.requestCount ?? 0)}
      </div>
      <div data-testid="total-tokens">
        {String(data?.accounts[0]?.totalTokens ?? 0)}
      </div>
      <div data-testid="in-progress">
        {String(data?.accounts[0]?.inProgressInvocationCount ?? 0)}
      </div>
      <div data-testid="first-recent-invoke">
        {data?.accounts[0]?.recentInvocations[0]?.invokeId ?? ""}
      </div>
    </div>
  );
}

describe("resolveUpstreamAccountRecentPreviewLimit", () => {
  it("clamps to the minimum when there is no in-flight activity", () => {
    expect(
      resolveUpstreamAccountRecentPreviewLimit([
        { inProgressInvocationCount: 0 },
        { inProgressInvocationCount: null },
      ]),
    ).toBe(DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MIN);
  });

  it("uses the highest in-progress account count", () => {
    expect(
      resolveUpstreamAccountRecentPreviewLimit([
        { inProgressInvocationCount: 3 },
        { inProgressInvocationCount: 9 },
        { inProgressInvocationCount: 5 },
      ]),
    ).toBe(9);
  });

  it("clamps to the configured maximum", () => {
    expect(
      resolveUpstreamAccountRecentPreviewLimit([
        { inProgressInvocationCount: 18 },
      ]),
    ).toBe(DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MAX);
  });
});

describe("useDashboardUpstreamAccountActivity", () => {
  it("does not immediately queue a second HTTP reconcile when records arrive before first hydration", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-04-04T10:05:00Z"));
    const initial = deferred<UpstreamAccountActivityResponse>();
    apiMocks.fetchUpstreamAccountActivity
      .mockImplementationOnce(async () => initial.promise)
      .mockResolvedValue(createAccountResponse(0, []));

    render(<Probe />);
    await flushAsync();
    expect(apiMocks.fetchUpstreamAccountActivity).toHaveBeenCalledTimes(1);

    emitRecords([
      createRecord({
        id: 9,
        invokeId: "pre-hydration-record",
        status: "success",
        totalTokens: 20,
      }),
    ]);

    initial.resolve(createAccountResponse(0, []));
    await flushAsync();
    expect(apiMocks.fetchUpstreamAccountActivity).toHaveBeenCalledTimes(1);

    await act(async () => {
      vi.advanceTimersByTime(4_999);
    });
    await flushAsync();
    expect(apiMocks.fetchUpstreamAccountActivity).toHaveBeenCalledTimes(1);

    await act(async () => {
      vi.advanceTimersByTime(1);
    });
    await flushAsync();
    expect(apiMocks.fetchUpstreamAccountActivity).toHaveBeenCalledTimes(2);
  });

  it("patches known account cards from SSE after the 1s visible batch while preserving the 5s HTTP budget", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-04-04T10:05:00Z"));
    apiMocks.fetchUpstreamAccountActivity.mockResolvedValue(
      createAccountResponse(
        1,
        [
          createPreview({
            id: 10,
            invokeId: "seed",
            occurredAt: "2026-04-04T10:04:00Z",
            status: "success",
            totalTokens: 100,
            upstreamAccountId: 42,
            upstreamAccountName: "Pool Alpha",
          }),
        ],
      ),
    );

    render(<Probe />);
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccountActivity).toHaveBeenCalledTimes(1);
    expect(text("request-count")).toBe("1");
    expect(text("total-tokens")).toBe("100");

    emitRecords([
      createRecord({
        id: 11,
        invokeId: "live-success",
        status: "success",
        createdAt: "2026-04-04T10:05:01Z",
        occurredAt: "2026-04-04T10:05:01Z",
        totalTokens: 80,
        cost: 0.08,
      }),
    ]);
    expect(text("request-count")).toBe("1");
    expect(apiMocks.fetchUpstreamAccountActivity).toHaveBeenCalledTimes(1);

    await act(async () => {
      vi.advanceTimersByTime(DASHBOARD_SSE_VISIBLE_PATCH_BATCH_MS);
    });

    expect(text("request-count")).toBe("2");
    expect(text("total-tokens")).toBe("180");
    expect(text("first-recent-invoke")).toBe("live-success");
    expect(apiMocks.fetchUpstreamAccountActivity).toHaveBeenCalledTimes(1);

    await act(async () => {
      vi.advanceTimersByTime(4_000);
    });
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccountActivity).toHaveBeenCalledTimes(2);
  });

  it("does not create incomplete cards for unseen accounts before reconcile", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-04-04T10:05:00Z"));
    apiMocks.fetchUpstreamAccountActivity.mockResolvedValue(
      createAccountResponse(0, []),
    );

    render(<Probe />);
    await flushAsync();

    emitRecords([
      createRecord({
        id: 90,
        invokeId: "new-account",
        status: "success",
        upstreamAccountId: 99,
        upstreamAccountName: "Pool New",
        totalTokens: 40,
      }),
    ]);

    await act(async () => {
      vi.advanceTimersByTime(DASHBOARD_SSE_VISIBLE_PATCH_BATCH_MS);
    });

    expect(text("request-count")).toBe("0");
    expect(apiMocks.fetchUpstreamAccountActivity).toHaveBeenCalledTimes(1);

    await act(async () => {
      vi.advanceTimersByTime(4_000);
    });
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccountActivity).toHaveBeenCalledTimes(2);
  });

  it("ignores out-of-range account records and replaces running contribution with terminal contribution", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-04-04T10:05:00Z"));
    apiMocks.fetchUpstreamAccountActivity.mockResolvedValue(
      createAccountResponse(1, []),
    );

    render(<Probe />);
    await flushAsync();

    emitRecords([
      createRecord({
        id: 120,
        invokeId: "older-account-record",
        status: "success",
        occurredAt: "2026-04-03T23:59:59Z",
        totalTokens: 40,
      }),
    ]);
    await act(async () => {
      vi.advanceTimersByTime(DASHBOARD_SSE_VISIBLE_PATCH_BATCH_MS);
    });

    expect(text("request-count")).toBe("0");
    expect(text("in-progress")).toBe("1");

    emitRecords([
      createRecord({
        id: 121,
        invokeId: "lifecycle-account-record",
        status: "running",
        poolAttemptCount: 2,
      }),
    ]);
    await act(async () => {
      vi.advanceTimersByTime(DASHBOARD_SSE_VISIBLE_PATCH_BATCH_MS);
    });
    expect(text("request-count")).toBe("0");
    expect(text("in-progress")).toBe("1");

    emitRecords([
      createRecord({
        id: 121,
        invokeId: "lifecycle-account-record",
        status: "success",
        totalTokens: 20,
        cost: 0.02,
      }),
    ]);
    await act(async () => {
      vi.advanceTimersByTime(DASHBOARD_SSE_VISIBLE_PATCH_BATCH_MS);
    });

    expect(text("request-count")).toBe("0");
    expect(text("total-tokens")).toBe("20");
    expect(text("in-progress")).toBe("0");
  });

  it("consumes hydrated account in-progress count when only the terminal record arrives later", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-04-04T10:05:00Z"));
    apiMocks.fetchUpstreamAccountActivity.mockResolvedValue(
      createAccountResponse(
        1,
        [
          createPreview({
            id: 130,
            invokeId: "prehydrated-account-running",
            occurredAt: "2026-04-04T10:04:30Z",
            status: "running",
            totalTokens: 0,
            upstreamAccountId: 42,
            upstreamAccountName: "Pool Alpha",
          }),
        ],
      ),
    );

    render(<Probe />);
    await flushAsync();

    emitRecords([
      createRecord({
        id: 130,
        invokeId: "prehydrated-account-running",
        status: "success",
        totalTokens: 20,
        cost: 0.02,
      }),
    ]);

    await act(async () => {
      vi.advanceTimersByTime(DASHBOARD_SSE_VISIBLE_PATCH_BATCH_MS);
    });

    expect(text("request-count")).toBe("1");
    expect(text("total-tokens")).toBe("20");
    expect(text("in-progress")).toBe("0");
  });

  it("expands the recent limit after hydration when an account has more in-flight invocations", async () => {
    const first = createAccountResponse(
      9,
      Array.from({ length: 4 }, (_, index) =>
        createPreview({
          id: 100 + index,
          invokeId: `seed-${index + 1}`,
          occurredAt: `2026-04-04T10:0${4 - index}:00Z`,
          status: index < 3 ? "running" : "success",
        }),
      ),
    );
    const second = createAccountResponse(
      9,
      Array.from({ length: 9 }, (_, index) =>
        createPreview({
          id: 200 + index,
          invokeId: `expanded-${index + 1}`,
          occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
          status: index < 7 ? "running" : "success",
        }),
      ),
    );

    apiMocks.fetchUpstreamAccountActivity
      .mockResolvedValueOnce(first)
      .mockResolvedValueOnce(second);

    render(<Probe />);
    await flushAsync();
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccountActivity).toHaveBeenCalledTimes(2);
    expect(apiMocks.fetchUpstreamAccountActivity.mock.calls[0]?.[1]).toEqual(
      expect.objectContaining({ recentLimit: 4 }),
    );
    expect(apiMocks.fetchUpstreamAccountActivity.mock.calls[1]?.[1]).toEqual(
      expect.objectContaining({ recentLimit: 9 }),
    );
    expect(text("visible-limit")).toBe("9");
    expect(text("recent-count")).toBe("9");
    expect(text("loading")).toBe("false");
  });

  it("does not shrink below an already discovered larger account limit", async () => {
    apiMocks.fetchUpstreamAccountActivity.mockResolvedValue(
      createAccountResponse(
        9,
        Array.from({ length: 9 }, (_, index) =>
          createPreview({
            id: 300 + index,
            invokeId: `stable-${index + 1}`,
            occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
            status: "running",
          }),
        ),
      ),
    );

    render(<Probe recentInvocationLimit={7} />);
    await flushAsync();
    await flushAsync();

    expect(text("visible-limit")).toBe("9");
  });

  it("does not shrink below the requested seed limit when the response resolves smaller", async () => {
    apiMocks.fetchUpstreamAccountActivity.mockResolvedValue(
      createAccountResponse(
        3,
        Array.from({ length: 7 }, (_, index) =>
          createPreview({
            id: 350 + index,
            invokeId: `seed-stable-${index + 1}`,
            occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
            status: index < 3 ? "running" : "success",
          }),
        ),
      ),
    );

    render(<Probe recentInvocationLimit={7} />);
    await flushAsync();
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccountActivity).toHaveBeenCalledTimes(1);
    expect(apiMocks.fetchUpstreamAccountActivity.mock.calls[0]?.[1]).toEqual(
      expect.objectContaining({ recentLimit: 7 }),
    );
    expect(text("visible-limit")).toBe("7");
    expect(text("recent-count")).toBe("7");
  });

  it("ignores stale smaller responses after a larger limit reload is queued", async () => {
    const first = deferred<UpstreamAccountActivityResponse>();
    const second = deferred<UpstreamAccountActivityResponse>();
    apiMocks.fetchUpstreamAccountActivity
      .mockImplementationOnce(async () => first.promise)
      .mockImplementationOnce(async () => second.promise);

    render(<Probe />);
    await flushAsync();

    first.resolve(
      createAccountResponse(
        9,
        Array.from({ length: 4 }, (_, index) =>
          createPreview({
            id: 400 + index,
            invokeId: `deferred-seed-${index + 1}`,
            occurredAt: `2026-04-04T10:0${4 - index}:00Z`,
            status: "running",
          }),
        ),
      ),
    );
    await flushAsync();

    second.resolve(
      createAccountResponse(
        9,
        Array.from({ length: 9 }, (_, index) =>
          createPreview({
            id: 500 + index,
            invokeId: `deferred-expanded-${index + 1}`,
            occurredAt: `2026-04-04T10:${String(59 - index).padStart(2, "0")}:00Z`,
            status: "running",
          }),
        ),
      ),
    );
    await flushAsync();

    expect(text("visible-limit")).toBe("9");
    expect(text("recent-count")).toBe("9");
    expect(text("error")).toBe("");
  });
});
