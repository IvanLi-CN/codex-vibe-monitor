/** @vitest-environment jsdom */
import type React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DashboardRecentNetworkWindowResponse } from "../lib/api";
import { useDashboardRecentNetworkWindow } from "./useDashboardRecentNetworkWindow";

const topicMocks = vi.hoisted(() => ({
  calls: [] as Array<{ descriptor: unknown; enabled: boolean }>,
  refresh: vi.fn(),
  state: {
    data: null as DashboardRecentNetworkWindowResponse | null,
    isLoading: false,
    error: null as string | null,
    lastReceivedAt: null as number | null,
  },
}));

vi.mock("./useSubscriptionTopic", () => ({
  useSubscriptionTopic: (descriptor: unknown, enabled: boolean) => {
    topicMocks.calls.push({ descriptor, enabled });
    return {
      data: topicMocks.state.data,
      isLoading: topicMocks.state.isLoading,
      error: topicMocks.state.error,
      lastReceivedAt: topicMocks.state.lastReceivedAt,
      refresh: topicMocks.refresh,
    };
  },
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeEach(() => {
  vi.useFakeTimers();
  topicMocks.calls = [];
  topicMocks.refresh.mockReset();
  topicMocks.state.data = null;
  topicMocks.state.isLoading = false;
  topicMocks.state.error = null;
  topicMocks.state.lastReceivedAt = null;
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  vi.useRealTimers();
});

function createResponse(): DashboardRecentNetworkWindowResponse {
  return {
    rangeStart: "2026-07-20T10:00:00.000Z",
    rangeEnd: "2026-07-20T10:05:00.000Z",
    windowSeconds: 300,
    sampleSeconds: 1,
    isWarmingUp: false,
    points: [
      {
        sampleStart: "2026-07-20T10:04:59.000Z",
        sampleEnd: "2026-07-20T10:05:00.000Z",
        uploadBytesPerSecond: 2048,
        downloadBytesPerSecond: 4096,
        uploadBytes: 2048,
        downloadBytes: 4096,
        isAvailable: true,
      },
    ],
  };
}

function render(ui: React.ReactNode) {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

function text(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`);
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing element: ${testId}`);
  }
  return element.textContent ?? "";
}

function Probe({ enabled = true }: { enabled?: boolean }) {
  const { data, isLoading, isRefreshing, isStale, error, reload } =
    useDashboardRecentNetworkWindow(enabled);
  return (
    <div>
      <div data-testid="loading">{isLoading ? "true" : "false"}</div>
      <div data-testid="refreshing">{isRefreshing ? "true" : "false"}</div>
      <div data-testid="stale">{isStale ? "true" : "false"}</div>
      <div data-testid="error">{error ?? ""}</div>
      <div data-testid="point-count">{String(data?.points.length ?? 0)}</div>
      <button type="button" data-testid="reload" onClick={() => void reload()} />
    </div>
  );
}

describe("useDashboardRecentNetworkWindow", () => {
  it("subscribes to the recent network topic and exposes cached topic state", () => {
    topicMocks.state.data = createResponse();
    topicMocks.state.lastReceivedAt = Date.now();

    render(<Probe />);

    expect(topicMocks.calls.at(-1)).toEqual({
      descriptor: {
        topic: "dashboard.network-recent.current",
        params: {},
      },
      enabled: true,
    });
    expect(text("loading")).toBe("false");
    expect(text("refreshing")).toBe("false");
    expect(text("stale")).toBe("false");
    expect(text("point-count")).toBe("1");
  });

  it("does not refresh from the frontend timer or manual reload", () => {
    topicMocks.state.data = createResponse();
    topicMocks.state.lastReceivedAt = Date.now();

    render(<Probe />);

    act(() => {
      vi.advanceTimersByTime(3_000);
    });
    expect(topicMocks.refresh).not.toHaveBeenCalled();

    render(<Probe enabled={false} />);

    act(() => {
      vi.advanceTimersByTime(2_000);
    });
    expect(topicMocks.calls.at(-1)).toEqual({
      descriptor: null,
      enabled: false,
    });
    expect(topicMocks.refresh).not.toHaveBeenCalled();

    topicMocks.state.isLoading = true;
    render(<Probe />);
    act(() => {
      const button = host?.querySelector('[data-testid="reload"]');
      if (!(button instanceof HTMLButtonElement)) {
        throw new Error("Missing reload button");
      }
      button.click();
    });

    expect(text("loading")).toBe("true");
    expect(topicMocks.refresh).not.toHaveBeenCalled();
  });

  it("marks cached data stale when no pushed payload arrives within the threshold", () => {
    const receivedAt = Date.now();
    topicMocks.state.data = createResponse();
    topicMocks.state.lastReceivedAt = receivedAt;

    render(<Probe />);
    expect(text("stale")).toBe("false");

    act(() => {
      vi.advanceTimersByTime(5_001);
    });

    expect(text("stale")).toBe("true");
  });
});
