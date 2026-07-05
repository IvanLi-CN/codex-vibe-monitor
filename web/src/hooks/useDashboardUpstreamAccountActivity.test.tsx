/** @vitest-environment jsdom */
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type {
  PromptCacheConversationInvocationPreview,
  UpstreamAccountActivityResponse,
} from "../lib/api";
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

vi.mock("../lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchUpstreamAccountActivity: apiMocks.fetchUpstreamAccountActivity,
  };
});

vi.mock("../lib/sse", () => ({
  subscribeToSse: () => () => {},
  subscribeToSseOpen: () => () => {},
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
