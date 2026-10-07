/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TaskTimelinePage, TaskTimelineSegment } from "../lib/api";
import { useManagedTaskTimeline } from "./useManagedTaskTimeline";

const mocks = vi.hoisted(() => ({
  fetchManagedTaskTimeline: vi.fn(),
  data: null as { watermark: number; observedAt: string } | null,
  error: null as string | null,
  refresh: vi.fn(),
}));

vi.mock("../lib/api", async () => ({
  ...(await vi.importActual<typeof import("../lib/api")>("../lib/api")),
  fetchManagedTaskTimeline: mocks.fetchManagedTaskTimeline,
}));
vi.mock("./useSubscriptionTopic", () => ({
  useSubscriptionTopic: () => ({
    data: mocks.data,
    error: mocks.error,
    isLoading: mocks.data == null,
    lastReceivedAt: mocks.data == null ? null : Date.now(),
    refresh: mocks.refresh,
  }),
}));

interface HookState {
  segments: TaskTimelineSegment[];
  coverage: TaskTimelinePage["coverage"];
  watermark: number | null;
  error: string | null;
  isLoading: boolean;
  refresh: () => void;
}

let host: HTMLDivElement | null = null;
let root: Root | null = null;
let current: HookState | null = null;

function Probe() {
  current = useManagedTaskTimeline();
  return null;
}

function renderProbe() {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root?.render(<Probe />));
}

function rerenderProbe() {
  act(() => root?.render(<Probe />));
}

function marker(watermark: number) {
  return { watermark, observedAt: new Date().toISOString() };
}

function segment(segmentId: string, revision: number): TaskTimelineSegment {
  const startedAt = new Date(Date.now() - 60_000).toISOString();
  const finishedAt = new Date(Date.now() - 58_000).toISOString();
  return {
    segmentId,
    kind: "execution",
    taskKey: "retention_archive",
    title: "数据保留与归档",
    startedAt,
    lastObservedAt: finishedAt,
    finishedAt,
    durationMs: 2_000,
    status: "success",
    triggerKind: "interval",
    executionClass: null,
    reason: null,
    retryAt: null,
    activeChildTaskKey: null,
    activeChildTitle: null,
    managedRunId: revision,
    sessionId: "test-session",
    revision,
  };
}

function page(
  watermark: number,
  segments: TaskTimelineSegment[],
  options: { nextCursor?: string | null; resetRequired?: boolean } = {},
): TaskTimelinePage {
  const now = Date.now();
  return {
    observedAt: new Date(now).toISOString(),
    windowStart: new Date(now - 12 * 60 * 60 * 1000).toISOString(),
    windowEnd: new Date(now).toISOString(),
    watermark,
    segments,
    coverage: [],
    nextCursor: options.nextCursor ?? null,
    resetRequired: options.resetRequired ?? false,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

beforeEach(() => {
  mocks.fetchManagedTaskTimeline.mockReset();
  mocks.data = marker(10);
  mocks.error = null;
  mocks.refresh.mockReset();
  current = null;
});

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  host = null;
  root = null;
  current = null;
});

describe("useManagedTaskTimeline", () => {
  it("commits a fixed-watermark baseline atomically and coalesces notices during paging", async () => {
    const secondBaselinePage = deferred<TaskTimelinePage>();
    mocks.fetchManagedTaskTimeline
      .mockResolvedValueOnce(page(10, [segment("run-one", 1)], { nextCursor: "baseline-next" }))
      .mockImplementationOnce(() => secondBaselinePage.promise)
      .mockResolvedValueOnce(page(12, [segment("run-one", 11)], { nextCursor: "delta-next" }))
      .mockResolvedValueOnce(page(12, [segment("run-one", 12), segment("run-two", 12)]));

    renderProbe();
    await vi.waitFor(() => expect(mocks.fetchManagedTaskTimeline).toHaveBeenCalledTimes(2));
    expect(current).toMatchObject({ segments: [], coverage: [], watermark: null });
    expect(mocks.fetchManagedTaskTimeline.mock.calls[0][0]).toEqual(
      expect.objectContaining({ limit: 500, from: expect.any(String), to: expect.any(String) }),
    );
    expect(mocks.fetchManagedTaskTimeline.mock.calls[1][0]).toEqual({
      cursor: "baseline-next",
      limit: 500,
    });

    mocks.data = marker(11);
    rerenderProbe();
    mocks.data = marker(12);
    rerenderProbe();
    expect(mocks.fetchManagedTaskTimeline).toHaveBeenCalledTimes(2);

    await act(async () => {
      secondBaselinePage.resolve(page(10, [segment("run-two", 2)]));
      await secondBaselinePage.promise;
    });
    await vi.waitFor(() => expect(current?.watermark).toBe(12));

    expect(mocks.fetchManagedTaskTimeline.mock.calls[2][0]).toEqual(
      expect.objectContaining({ afterRevision: 10, limit: 500 }),
    );
    expect(mocks.fetchManagedTaskTimeline.mock.calls[3][0]).toEqual({
      cursor: "delta-next",
      limit: 500,
    });
    expect(
      mocks.fetchManagedTaskTimeline.mock.calls.filter(([query]) => query.afterRevision != null),
    ).toHaveLength(1);
    expect(current?.segments).toHaveLength(2);
    expect(current?.segments.find((item) => item.segmentId === "run-one")?.revision).toBe(12);
    expect(current?.segments.find((item) => item.segmentId === "run-two")?.revision).toBe(12);
  });

  it("restarts an incomplete baseline when its cursor expires", async () => {
    mocks.fetchManagedTaskTimeline
      .mockResolvedValueOnce(page(10, [segment("partial", 1)], { nextCursor: "expired" }))
      .mockResolvedValueOnce(page(10, [], { resetRequired: true }))
      .mockResolvedValueOnce(page(11, [segment("complete", 2)]));

    renderProbe();
    await vi.waitFor(() => expect(current?.watermark).toBe(11));

    expect(mocks.fetchManagedTaskTimeline).toHaveBeenCalledTimes(3);
    expect(current?.segments.map((item) => item.segmentId)).toEqual(["complete"]);
    expect(current?.error).toBeNull();
  });

  it("retains the last complete data after an HTTP failure and retries on a new notice", async () => {
    mocks.fetchManagedTaskTimeline
      .mockResolvedValueOnce(page(10, [segment("kept", 1)]))
      .mockRejectedValueOnce(new Error("maintenance store unavailable"))
      .mockResolvedValueOnce(page(11, [segment("kept", 2)]));

    renderProbe();
    await vi.waitFor(() => expect(current?.watermark).toBe(10));

    mocks.data = marker(11);
    rerenderProbe();
    await vi.waitFor(() => expect(current?.error).toContain("maintenance store unavailable"));
    expect(current).toMatchObject({
      watermark: 10,
      segments: [expect.objectContaining({ revision: 1 })],
    });

    mocks.data = marker(11);
    rerenderProbe();
    await vi.waitFor(() => expect(current?.watermark).toBe(11));
    expect(current?.segments).toEqual([
      expect.objectContaining({ segmentId: "kept", revision: 2 }),
    ]);
    expect(current?.error).toBeNull();
  });
});
