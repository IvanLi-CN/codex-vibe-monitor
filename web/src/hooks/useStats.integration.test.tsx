/** @vitest-environment jsdom */
import type React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiRequestError, type StatsResponse } from "../lib/api";
import { clearSummaryRemountCache, useSummary } from "./useStats";

const apiMocks = vi.hoisted(() => ({
  fetchSummary: vi.fn<() => Promise<StatsResponse>>(),
}));

const topicMocks = vi.hoisted(() => ({
  state: {
    data: null as StatsResponse | null,
    isLoading: false,
    error: null as string | null,
    refresh: vi.fn(),
  },
  lastDescriptor: null as Record<string, unknown> | null,
  lastEnabled: true,
}));

vi.mock("../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchSummary: apiMocks.fetchSummary,
  };
});

vi.mock("./useSubscriptionTopic", () => ({
  useSubscriptionTopic: (descriptor: Record<string, unknown> | null, enabled = true) => {
    topicMocks.lastDescriptor = descriptor;
    topicMocks.lastEnabled = enabled;
    return topicMocks.state;
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

beforeEach(() => {
  topicMocks.state.data = null;
  topicMocks.state.isLoading = false;
  topicMocks.state.error = null;
  topicMocks.state.refresh.mockReset();
  topicMocks.lastDescriptor = null;
  topicMocks.lastEnabled = true;
  apiMocks.fetchSummary.mockReset();
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  clearSummaryRemountCache();
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
  });
}

function text(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`);
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing element: ${testId}`);
  }
  return element.textContent ?? "";
}

function Probe({ window }: { window: string }) {
  const { summary, isLoading, error, errorCode } = useSummary(window);
  return (
    <div>
      <div data-testid="total">{String(summary?.totalCount ?? 0)}</div>
      <div data-testid="loading">{isLoading ? "true" : "false"}</div>
      <div data-testid="error">{error ?? ""}</div>
      <div data-testid="error-code">{errorCode ?? ""}</div>
    </div>
  );
}

describe("useSummary", () => {
  it("subscribes to stats.summary.current for open windows", () => {
    topicMocks.state.data = {
      totalCount: 11,
      successCount: 10,
      failureCount: 1,
      totalCost: 1.1,
      totalTokens: 1100,
    } as StatsResponse;

    render(<Probe window="current" />);

    expect(topicMocks.lastDescriptor).toEqual({
      topic: "stats.summary.current",
      params: expect.objectContaining({
        window: "current",
      }),
    });
    expect(topicMocks.lastEnabled).toBe(true);
    expect(text("total")).toBe("11");
  });

  it("uses HTTP for yesterday summaries", async () => {
    apiMocks.fetchSummary.mockResolvedValue({
      totalCount: 7,
      successCount: 6,
      failureCount: 1,
      totalCost: 0.7,
      totalTokens: 700,
    } as StatsResponse);

    render(<Probe window="yesterday" />);
    await flushAsync();

    expect(topicMocks.lastDescriptor).toBeNull();
    expect(topicMocks.lastEnabled).toBe(false);
    expect(apiMocks.fetchSummary).toHaveBeenCalledTimes(1);
    expect(text("total")).toBe("7");
  });

  it("exposes coded HTTP recovery failures to the dashboard", async () => {
    apiMocks.fetchSummary.mockRejectedValue(
      new ApiRequestError(503, "Request failed: 503", {
        code: "summary_recovery_pending",
      }),
    );

    render(<Probe window="yesterday" />);
    await flushAsync();

    expect(text("loading")).toBe("false");
    expect(text("error")).toContain("503");
    expect(text("error-code")).toBe("summary_recovery_pending");
    expect(text("total")).toBe("0");
  });

  it("retries a pending HTTP recovery summary until it becomes exact", async () => {
    vi.useFakeTimers();
    apiMocks.fetchSummary
      .mockRejectedValueOnce(
        new ApiRequestError(503, "Request failed: 503", {
          code: "summary_recovery_pending",
        }),
      )
      .mockResolvedValueOnce({
        totalCount: 9,
        successCount: 8,
        failureCount: 1,
        totalCost: 0.9,
        totalTokens: 900,
      } as StatsResponse);

    try {
      render(<Probe window="yesterday" />);
      await flushAsync();
      expect(apiMocks.fetchSummary).toHaveBeenCalledTimes(1);
      expect(text("error-code")).toBe("summary_recovery_pending");

      await act(async () => {
        await vi.advanceTimersByTimeAsync(1_000);
      });
      expect(apiMocks.fetchSummary).toHaveBeenCalledTimes(2);
      expect(text("total")).toBe("9");
      expect(text("error-code")).toBe("");
    } finally {
      vi.useRealTimers();
    }
  });

  it("uses HTTP for previous7d summaries", async () => {
    apiMocks.fetchSummary.mockResolvedValue({
      totalCount: 17,
      successCount: 14,
      failureCount: 3,
      totalCost: 1.7,
      totalTokens: 1700,
    } as StatsResponse);

    render(<Probe window="previous7d" />);
    await flushAsync();

    expect(topicMocks.lastDescriptor).toBeNull();
    expect(topicMocks.lastEnabled).toBe(false);
    expect(apiMocks.fetchSummary).toHaveBeenCalledTimes(1);
    expect(text("total")).toBe("17");
  });

  it("refreshes previous7d over HTTP at each local midnight", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 3, 8, 23, 59, 58));
    apiMocks.fetchSummary
      .mockResolvedValueOnce({
        totalCount: 17,
        successCount: 14,
        failureCount: 3,
        totalCost: 1.7,
        totalTokens: 1700,
      } as StatsResponse)
      .mockResolvedValueOnce({
        totalCount: 18,
        successCount: 15,
        failureCount: 3,
        totalCost: 1.8,
        totalTokens: 1800,
      } as StatsResponse)
      .mockResolvedValueOnce({
        totalCount: 19,
        successCount: 16,
        failureCount: 3,
        totalCost: 1.9,
        totalTokens: 1900,
      } as StatsResponse);

    try {
      render(<Probe window="previous7d" />);
      await flushAsync();

      expect(apiMocks.fetchSummary).toHaveBeenCalledTimes(1);
      expect(text("total")).toBe("17");

      await act(async () => {
        vi.advanceTimersByTime(2_100);
        await Promise.resolve();
        await Promise.resolve();
      });

      expect(apiMocks.fetchSummary).toHaveBeenCalledTimes(2);
      expect(text("total")).toBe("18");

      await act(async () => {
        await vi.advanceTimersByTimeAsync(24 * 60 * 60 * 1000);
      });

      expect(apiMocks.fetchSummary).toHaveBeenCalledTimes(3);
      expect(text("total")).toBe("19");
    } finally {
      vi.useRealTimers();
    }
  });
});
