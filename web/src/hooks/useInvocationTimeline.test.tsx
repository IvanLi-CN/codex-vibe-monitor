/** @vitest-environment jsdom */

import type React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { InvocationTimelineResponse, TimeseriesResponse } from "../lib/api";
import { useInvocationTimeline } from "./useInvocationTimeline";

const timelineMocks = vi.hoisted(() => ({
  fetch: vi.fn(),
  release: vi.fn(),
}));

vi.mock("../lib/api", () => ({
  fetchInvocationTimeline: timelineMocks.fetch,
  releaseInvocationTimelineSnapshot: timelineMocks.release,
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

function createTimeseries(rangeStart: string, rangeEnd: string): TimeseriesResponse {
  return {
    rangeStart,
    rangeEnd,
    bucketSeconds: 300,
    points: [],
  };
}

function createTimeline(
  invokeId: string,
  rangeStart = "2026-07-16T10:00:00.000Z",
  rangeEnd = "2026-07-16T10:30:00.000Z",
): InvocationTimelineResponse {
  return {
    rangeStart,
    rangeEnd,
    asOf: rangeEnd,
    total: 1,
    hasMore: false,
    nextCursor: null,
    records: [
      {
        id: 1,
        invokeId,
        occurredAt: "2026-07-16T10:15:00.000Z",
        endAt: "2026-07-16T10:16:00.000Z",
        isInFlight: false,
        status: "success",
        tTotalMs: 60_000,
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

async function advanceTime(milliseconds: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(milliseconds);
    await Promise.resolve();
  });
}

function Probe({
  response,
  closedNaturalDay = false,
  liveRevision,
  liveRefreshAllowed = true,
}: {
  response: TimeseriesResponse | null;
  closedNaturalDay?: boolean;
  liveRevision?: number;
  liveRefreshAllowed?: boolean;
}) {
  const { data, bounds, error, isLoading, isRefreshing, isStale, window, setWindow } =
    useInvocationTimeline({
      response,
      closedNaturalDay,
      liveRevision,
      liveRefreshAllowed,
    });
  return (
    <>
      <output data-testid="invoke-id">{data?.records[0]?.invokeId ?? ""}</output>
      <output data-testid="record-count">{data?.records.length ?? 0}</output>
      <output data-testid="record-ids">
        {data?.records.map((record) => record.invokeId).join(",") ?? ""}
      </output>
      <output data-testid="error">{error ?? ""}</output>
      <output data-testid="loading">{String(isLoading)}</output>
      <output data-testid="refreshing">{String(isRefreshing)}</output>
      <output data-testid="stale">{String(isStale)}</output>
      <output data-testid="window">
        {window
          ? `${new Date(window.startMs).toISOString()}|${new Date(window.endMs).toISOString()}`
          : ""}
      </output>
      <button
        type="button"
        data-testid="zoom-window"
        onClick={() => {
          if (!bounds) return;
          setWindow({
            startMs: bounds.startMs + 60_000,
            endMs: bounds.endMs,
          });
        }}
      />
    </>
  );
}

describe("useInvocationTimeline", () => {
  beforeEach(() => {
    timelineMocks.fetch.mockReset();
    timelineMocks.release.mockReset().mockResolvedValue(undefined);
  });

  afterEach(() => {
    act(() => {
      root?.unmount();
    });
    host?.remove();
    host = null;
    root = null;
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  it("invalidates and aborts a pending request when the timeline context changes", async () => {
    const pending: Array<{
      resolve: (value: InvocationTimelineResponse) => void;
      signal?: AbortSignal;
    }> = [];
    timelineMocks.fetch.mockImplementation((_options: { signal?: AbortSignal }) => {
      return new Promise<InvocationTimelineResponse>((resolve) => {
        pending.push({ resolve, signal: _options.signal });
      });
    });

    const firstResponse = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");
    const secondResponse = createTimeseries("2026-07-17T10:00:00.000Z", "2026-07-17T10:30:00.000Z");

    render(<Probe response={firstResponse} />);
    const initialRequestCount = pending.length;
    expect(initialRequestCount).toBeGreaterThan(0);
    const firstSignal = pending[0]?.signal;

    act(() => {
      root?.render(<Probe response={secondResponse} />);
    });
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(firstSignal?.aborted).toBe(true);
    expect(pending.length).toBe(initialRequestCount + 1);
    for (const request of pending.slice(0, initialRequestCount)) {
      request.resolve(createTimeline("stale-invoke"));
    }
    await act(async () => {
      await Promise.resolve();
    });
    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("");

    pending
      .at(-1)
      ?.resolve(
        createTimeline("current-invoke", "2026-07-17T10:00:00.000Z", "2026-07-17T10:30:00.000Z"),
      );
    await act(async () => {
      await Promise.resolve();
    });
    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("current-invoke");
  });

  it("waits for valid timeseries bounds before the initial request", async () => {
    timelineMocks.fetch.mockResolvedValue(createTimeline("bounded-window"));

    render(<Probe response={null} />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(timelineMocks.fetch).not.toHaveBeenCalled();

    const response = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");
    act(() => {
      root?.render(<Probe response={response} />);
    });
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(timelineMocks.fetch).toHaveBeenCalledTimes(1);
  });

  it("loads the initial snapshot even when live refresh is unavailable", async () => {
    timelineMocks.fetch.mockResolvedValue(createTimeline("offline-initial"));
    const response = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");

    render(<Probe response={response} liveRefreshAllowed={false} />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(timelineMocks.fetch).toHaveBeenCalledTimes(1);
    expect(timelineMocks.fetch.mock.calls[0]?.[0]).toMatchObject({
      naturalDayStart: "2026-07-16T10:00:00.000Z",
      naturalDayEnd: "2026-07-16T10:30:00.000Z",
    });
    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("offline-initial");
  });

  it("aborts and invalidates a request when timeseries bounds disappear", async () => {
    const pending: Array<{
      resolve: (value: InvocationTimelineResponse) => void;
      signal?: AbortSignal;
    }> = [];
    timelineMocks.fetch.mockImplementation((_options: { signal?: AbortSignal }) => {
      return new Promise<InvocationTimelineResponse>((resolve) => {
        pending.push({ resolve, signal: _options.signal });
      });
    });
    const response = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");

    render(<Probe response={response} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(pending).toHaveLength(1);

    act(() => {
      root?.render(<Probe response={null} />);
    });
    expect(pending[0]?.signal?.aborted).toBe(true);
    pending[0]?.resolve(createTimeline("stale-bounds"));
    await act(async () => {
      await Promise.resolve();
    });

    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("");
  });

  it("does not refresh a closed day when the live revision changes", async () => {
    timelineMocks.fetch.mockResolvedValue(createTimeline("closed-day"));
    const response = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");

    render(<Probe response={response} closedNaturalDay liveRevision={1} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(1);

    act(() => {
      root?.render(<Probe response={response} closedNaturalDay liveRevision={2} />);
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(timelineMocks.fetch).toHaveBeenCalledTimes(1);
  });

  it("refreshes when a live revision changes within the same time bounds", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-16T10:30:00.000Z"));
    timelineMocks.fetch.mockResolvedValue(createTimeline("stable-window"));
    const firstResponse = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");
    const nextResponse = { ...firstResponse, points: [] };

    render(<Probe response={firstResponse} liveRevision={1} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(1);

    act(() => {
      root?.render(<Probe response={nextResponse} liveRevision={2} />);
    });
    await advanceTime(15_050);

    expect(timelineMocks.fetch).toHaveBeenCalledTimes(2);
  });

  it("coalesces live revisions while one snapshot traversal is in flight", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-16T10:30:00.000Z"));
    const pending: Array<(value: InvocationTimelineResponse) => void> = [];
    timelineMocks.fetch.mockImplementation(
      () => new Promise<InvocationTimelineResponse>((resolve) => pending.push(resolve)),
    );
    const response = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");

    render(<Probe response={response} liveRevision={1} />);
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(1);
    act(() => {
      root?.render(<Probe response={response} liveRevision={2} />);
    });
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(1);

    pending.shift()?.(createTimeline("revision-1"));
    await act(async () => {
      await Promise.resolve();
    });
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(1);
    await advanceTime(15_050);
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(2);
    pending.shift()?.(createTimeline("revision-2"));
    await act(async () => {
      await Promise.resolve();
    });
    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("revision-2");
  });

  it("does not run a coalesced follow-up while live refresh is frozen", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-16T10:30:00.000Z"));
    const pending: Array<(value: InvocationTimelineResponse) => void> = [];
    timelineMocks.fetch.mockImplementation(
      () => new Promise<InvocationTimelineResponse>((resolve) => pending.push(resolve)),
    );
    const response = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");

    render(<Probe response={response} liveRevision={1} liveRefreshAllowed />);
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(1);
    act(() => {
      root?.render(<Probe response={response} liveRevision={2} liveRefreshAllowed />);
    });
    act(() => {
      root?.render(<Probe response={response} liveRevision={2} liveRefreshAllowed={false} />);
    });
    pending.shift()?.(createTimeline("frozen-request"));
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(1);

    act(() => {
      root?.render(<Probe response={response} liveRevision={2} liveRefreshAllowed />);
    });
    await advanceTime(15_050);
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(2);
  });

  it("limits SSE and polling to one traversal per rolling 15 seconds", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-16T10:30:00.000Z"));
    const startTimes: number[] = [];
    timelineMocks.fetch.mockImplementation(() => {
      startTimes.push(Date.now());
      return Promise.resolve(createTimeline(`revision-${startTimes.length}`));
    });
    const response = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");

    render(<Probe response={response} liveRevision={0} />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    for (let revision = 1; revision <= 100; revision += 1) {
      act(() => {
        root?.render(<Probe response={response} liveRevision={revision} />);
      });
      await advanceTime(1_200);
    }

    expect(startTimes.length).toBeLessThanOrEqual(9);
    expect(
      startTimes.every(
        (startedAt, index) => index === 0 || startedAt - startTimes[index - 1]! >= 15_000,
      ),
    ).toBe(true);
  });

  it("backs failed refreshes off by 15, 30, and 60 seconds, then resets on success", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-16T10:30:00.000Z"));
    const random = vi.spyOn(Math, "random").mockReturnValue(0.5);
    timelineMocks.fetch
      .mockResolvedValueOnce(createTimeline("last-good"))
      .mockRejectedValueOnce(new Error("first refresh failed"))
      .mockRejectedValueOnce(new Error("second refresh failed"))
      .mockRejectedValueOnce(new Error("third refresh failed"))
      .mockResolvedValueOnce(createTimeline("recovered"));
    const response = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");

    render(<Probe response={response} liveRevision={0} />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    act(() => {
      root?.render(<Probe response={response} liveRevision={1} />);
    });

    await advanceTime(15_050);
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(2);
    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("last-good");
    expect(host?.querySelector("[data-testid=stale]")?.textContent).toBe("true");

    await advanceTime(15_050);
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(3);
    await advanceTime(30_000);
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(4);
    await advanceTime(60_000);
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(5);
    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("recovered");
    expect(host?.querySelector("[data-testid=stale]")?.textContent).toBe("false");
    random.mockRestore();
  });

  it("traverses immutable pages and folds duplicate logical invocations", async () => {
    const firstPage = createTimeline("page-one");
    firstPage.asOf = "snapshot-1";
    firstPage.hasMore = true;
    firstPage.nextCursor = "cursor-1";
    firstPage.total = 2;
    const secondPage = createTimeline("page-one");
    secondPage.asOf = "snapshot-1";
    secondPage.records[0] = {
      ...secondPage.records[0],
      id: 2,
      status: "error",
    };
    secondPage.records.push({
      ...secondPage.records[0],
      id: 3,
      invokeId: "page-two",
      occurredAt: "2026-07-16T10:20:00.000Z",
    });
    secondPage.hasMore = false;
    secondPage.nextCursor = null;
    let resolveSecondPage: ((page: InvocationTimelineResponse) => void) | null = null;
    timelineMocks.fetch.mockResolvedValueOnce(firstPage).mockImplementationOnce(
      () =>
        new Promise<InvocationTimelineResponse>((resolve) => {
          resolveSecondPage = resolve;
        }),
    );
    const response = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");

    render(<Probe response={response} closedNaturalDay />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(timelineMocks.fetch).toHaveBeenCalledTimes(2);
    expect(timelineMocks.release).not.toHaveBeenCalled();
    expect(timelineMocks.fetch.mock.calls[1]?.[0]).toMatchObject({
      cursor: "cursor-1",
      asOf: "snapshot-1",
    });

    await act(async () => {
      resolveSecondPage?.(secondPage);
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(host?.querySelector("[data-testid=record-count]")?.textContent).toBe("2");
    expect(host?.querySelector("[data-testid=record-ids]")?.textContent).toBe("page-one,page-two");
    expect(timelineMocks.release).toHaveBeenCalledTimes(1);
    expect(timelineMocks.release).toHaveBeenCalledWith("snapshot-1");
  });

  it("rejects a truncated final snapshot instead of committing incomplete data", async () => {
    const truncated = createTimeline("only-record");
    truncated.total = 2;
    truncated.hasMore = false;
    timelineMocks.fetch.mockResolvedValue(truncated);
    const response = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");

    render(<Probe response={response} closedNaturalDay />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(host?.querySelector("[data-testid=record-count]")?.textContent).toBe("0");
    expect(host?.querySelector("[data-testid=error]")?.textContent).toBe(
      "Invocation timeline snapshot is incomplete",
    );
    expect(timelineMocks.release).not.toHaveBeenCalled();
  });

  it("keeps the last successful snapshot when a refresh fails", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-16T10:30:00.000Z"));
    timelineMocks.fetch.mockResolvedValueOnce(createTimeline("last-good"));
    timelineMocks.fetch.mockRejectedValueOnce(new Error("snapshot unavailable"));
    const response = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");

    render(<Probe response={response} liveRevision={1} />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("last-good");

    act(() => {
      root?.render(<Probe response={response} liveRevision={2} />);
    });
    await advanceTime(18_000);
    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("last-good");
    expect(host?.querySelector("[data-testid=stale]")?.textContent).toBe("true");
  });

  it("commits a complete snapshot even when releasing its server reservation fails", async () => {
    timelineMocks.fetch.mockResolvedValue(createTimeline("complete-snapshot"));
    timelineMocks.release.mockRejectedValue(new Error("release unavailable"));
    const debug = vi.spyOn(console, "debug").mockImplementation(() => undefined);
    const response = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");

    render(<Probe response={response} />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(timelineMocks.release).toHaveBeenCalledWith("2026-07-16T10:30:00.000Z");
    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("complete-snapshot");
    expect(debug).toHaveBeenCalled();
  });

  it("continues a failed-snapshot retry during SSE disconnection", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-16T10:30:00.000Z"));
    vi.spyOn(Math, "random").mockReturnValue(0.5);
    timelineMocks.fetch
      .mockResolvedValueOnce(createTimeline("last-good"))
      .mockRejectedValueOnce(new Error("temporary failure"))
      .mockResolvedValueOnce(createTimeline("recovered"));
    const response = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");

    render(<Probe response={response} liveRevision={1} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(1);

    act(() => {
      root?.render(<Probe response={response} liveRevision={2} />);
    });
    await advanceTime(15_050);
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(2);

    act(() => {
      root?.render(<Probe response={response} liveRevision={2} liveRefreshAllowed={false} />);
    });
    await advanceTime(15_049);
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(2);
    await advanceTime(1);

    expect(timelineMocks.fetch).toHaveBeenCalledTimes(3);
    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("recovered");
    expect(host?.querySelector("[data-testid=stale]")?.textContent).toBe("false");
  });

  it("uses one request when the live time bounds advance", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-16T10:30:00.000Z"));
    timelineMocks.fetch
      .mockResolvedValueOnce(createTimeline("initial-window"))
      .mockResolvedValueOnce(
        createTimeline("advancing-window", "2026-07-16T10:01:00.000Z", "2026-07-16T10:31:00.000Z"),
      );
    const firstResponse = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");
    const nextResponse = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:31:00.000Z");

    render(<Probe response={firstResponse} liveRevision={1} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(1);

    act(() => {
      root?.render(<Probe response={nextResponse} liveRevision={2} />);
    });
    await advanceTime(15_050);

    expect(timelineMocks.fetch).toHaveBeenCalledTimes(2);
  });

  it("keeps the committed snapshot and its window visible during a delayed live refresh", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-16T10:30:00.000Z"));
    const pending: Array<(value: InvocationTimelineResponse) => void> = [];
    timelineMocks.fetch.mockImplementation(
      () => new Promise<InvocationTimelineResponse>((resolve) => pending.push(resolve)),
    );
    const firstResponse = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");
    const nextResponse = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:31:00.000Z");

    render(<Probe response={firstResponse} liveRevision={1} />);
    expect(pending).toHaveLength(1);
    pending[0]?.(createTimeline("committed-window"));
    await act(async () => {
      await Promise.resolve();
    });

    act(() => {
      root?.render(<Probe response={nextResponse} liveRevision={2} />);
    });
    await advanceTime(15_050);
    expect(pending).toHaveLength(2);
    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("committed-window");
    expect(host?.querySelector("[data-testid=window]")?.textContent).toBe(
      "2026-07-16T10:00:00.000Z|2026-07-16T10:30:00.000Z",
    );
    expect(host?.querySelector("[data-testid=loading]")?.textContent).toBe("false");
    expect(host?.querySelector("[data-testid=refreshing]")?.textContent).toBe("true");

    pending[1]?.(
      createTimeline("replacement-window", "2026-07-16T10:01:00.000Z", "2026-07-16T10:31:00.000Z"),
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("replacement-window");
    expect(host?.querySelector("[data-testid=window]")?.textContent).toBe(
      "2026-07-16T10:01:00.000Z|2026-07-16T10:31:00.000Z",
    );
    expect(host?.querySelector("[data-testid=refreshing]")?.textContent).toBe("false");
  });

  it("keeps the committed live window visible and stale when its replacement fails", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-16T10:30:00.000Z"));
    timelineMocks.fetch.mockResolvedValueOnce(createTimeline("last-good"));
    const response = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");
    render(<Probe response={response} liveRevision={1} />);
    await act(async () => {
      await Promise.resolve();
    });

    timelineMocks.fetch.mockRejectedValueOnce(new Error("snapshot unavailable"));
    const advanced = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:31:00.000Z");
    act(() => {
      root?.render(<Probe response={advanced} liveRevision={2} />);
    });
    await advanceTime(15_050);

    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("last-good");
    expect(host?.querySelector("[data-testid=window]")?.textContent).toBe(
      "2026-07-16T10:00:00.000Z|2026-07-16T10:30:00.000Z",
    );
    expect(host?.querySelector("[data-testid=loading]")?.textContent).toBe("false");
    expect(host?.querySelector("[data-testid=stale]")?.textContent).toBe("true");
  });

  it("does not expose the previous window data while a zoom request is pending", async () => {
    const pending: Array<(value: InvocationTimelineResponse) => void> = [];
    timelineMocks.fetch.mockImplementation(
      () => new Promise<InvocationTimelineResponse>((resolve) => pending.push(resolve)),
    );
    const response = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");

    render(<Probe response={response} />);
    pending[0]?.(createTimeline("old-window"));
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("old-window");

    act(() => {
      host?.querySelector<HTMLButtonElement>("[data-testid=zoom-window]")?.click();
    });
    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("");
  });

  it("refreshes the latest viewport after a zoom during an in-flight request", async () => {
    const pending: Array<{
      resolve: (value: InvocationTimelineResponse) => void;
      options: { from?: string; to?: string };
      signal?: AbortSignal;
    }> = [];
    timelineMocks.fetch.mockImplementation(
      (options: { from?: string; to?: string; signal?: AbortSignal }) => {
        return new Promise<InvocationTimelineResponse>((resolve) =>
          pending.push({ resolve, options, signal: options.signal }),
        );
      },
    );
    const response = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");

    render(<Probe response={response} />);
    expect(pending).toHaveLength(1);
    act(() => {
      host?.querySelector<HTMLButtonElement>("[data-testid=zoom-window]")?.click();
    });
    await act(async () => {
      await Promise.resolve();
    });
    expect(pending).toHaveLength(2);
    expect(pending[0]?.signal?.aborted).toBe(true);
    expect(pending[1]?.options).toMatchObject({
      from: "2026-07-16T10:01:00.000Z",
      to: "2026-07-16T10:30:00.000Z",
    });

    pending[0]?.resolve(createTimeline("old-viewport"));
    pending[1]?.resolve(createTimeline("new-viewport", "2026-07-16T10:01:00.000Z"));
    await act(async () => {
      await Promise.resolve();
    });
    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("new-viewport");
  });

  it("accepts whole-second server bounds for fractional-second windows", async () => {
    timelineMocks.fetch.mockResolvedValue(
      createTimeline("fractional-window", "2026-07-16T10:00:00.000Z", "2026-07-16T10:20:01.000Z"),
    );
    const response = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:20:01.500Z");

    render(<Probe response={response} />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(host?.querySelector("[data-testid=invoke-id]")?.textContent).toBe("fractional-window");
  });

  it("refreshes after the minimum interval following a reconnect", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-16T10:30:00.000Z"));
    timelineMocks.fetch.mockResolvedValue(createTimeline("reconnected-window"));
    const firstResponse = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:30:00.000Z");
    const nextResponse = createTimeseries("2026-07-16T10:00:00.000Z", "2026-07-16T10:31:00.000Z");

    render(<Probe response={firstResponse} liveRevision={1} liveRefreshAllowed />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(1);

    act(() => {
      root?.render(<Probe response={nextResponse} liveRevision={2} liveRefreshAllowed={false} />);
    });
    await act(async () => {
      await Promise.resolve();
    });
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(1);

    act(() => {
      root?.render(<Probe response={nextResponse} liveRevision={2} liveRefreshAllowed />);
    });
    await advanceTime(15_050);
    expect(timelineMocks.fetch).toHaveBeenCalledTimes(2);
  });
});
