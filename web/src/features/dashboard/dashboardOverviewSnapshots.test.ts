import { afterEach, describe, expect, it, vi } from "vitest";
import {
  createDashboardOverviewSnapshotEntry,
  fetchDashboardOverviewSnapshotBundle,
  installDashboardOverviewSnapshotTestDriver,
  listDashboardOverviewSnapshotRanges,
  readDashboardOverviewSnapshotEntry,
  writeDashboardOverviewSnapshotEntry,
} from "./dashboardOverviewSnapshots";

const apiMocks = vi.hoisted(() => ({
  fetchDashboardActivity: vi.fn(),
  fetchDashboardNetworkTimeseries: vi.fn(),
  fetchParallelWorkStats: vi.fn(),
  fetchSummary: vi.fn(),
  fetchTimeseries: vi.fn(),
}));

vi.mock("../../lib/api", () => {
  class ApiRequestError extends Error {
    readonly status: number;

    constructor(status: number, message: string) {
      super(message);
      this.name = "ApiRequestError";
      this.status = status;
    }
  }

  return {
    ApiRequestError,
    fetchDashboardActivity: apiMocks.fetchDashboardActivity,
    fetchDashboardNetworkTimeseries: apiMocks.fetchDashboardNetworkTimeseries,
    fetchParallelWorkStats: apiMocks.fetchParallelWorkStats,
    fetchSummary: apiMocks.fetchSummary,
    fetchTimeseries: apiMocks.fetchTimeseries,
  };
});

function createMemoryDriver(initial: Record<string, unknown> = {}) {
  const store = new Map<string, unknown>(Object.entries(initial));
  return {
    get: async (range: string) => store.get(range) ?? null,
    put: async (entry: { range: string }) => {
      store.set(entry.range, entry);
    },
    list: async () => [...store.values()],
  };
}

afterEach(() => {
  installDashboardOverviewSnapshotTestDriver(null);
  vi.clearAllMocks();
});

describe("dashboardOverviewSnapshots store", () => {
  it("writes and overwrites the latest snapshot entry for a range", async () => {
    installDashboardOverviewSnapshotTestDriver(createMemoryDriver());

    const first = createDashboardOverviewSnapshotEntry(
      "today",
      {
        range: "today",
        timeseries: { rangeStart: "a", rangeEnd: "b", bucketSeconds: 60, points: [] },
      },
      "2026-07-17T04:00:00.000Z",
    );
    const second = createDashboardOverviewSnapshotEntry(
      "today",
      {
        range: "today",
        timeseries: { rangeStart: "c", rangeEnd: "d", bucketSeconds: 60, points: [] },
      },
      "2026-07-17T05:00:00.000Z",
    );

    await writeDashboardOverviewSnapshotEntry(first);
    await writeDashboardOverviewSnapshotEntry(second);

    await expect(readDashboardOverviewSnapshotEntry("today")).resolves.toEqual(second);
    await expect(listDashboardOverviewSnapshotRanges()).resolves.toEqual(["today"]);
  });

  it("filters invalid schema entries and returns null for uncached ranges", async () => {
    installDashboardOverviewSnapshotTestDriver(
      createMemoryDriver({
        today: {
          schemaVersion: 999,
          range: "today",
          cachedAt: "2026-07-17T04:00:00.000Z",
          payload: { range: "today" },
        },
        usage: {
          schemaVersion: 1,
          range: "usage",
          cachedAt: "2026-07-17T06:00:00.000Z",
          payload: {
            range: "usage",
            timeseries: { rangeStart: "a", rangeEnd: "b", bucketSeconds: 86400, points: [] },
          },
        },
      }),
    );

    await expect(readDashboardOverviewSnapshotEntry("today")).resolves.toBeNull();
    await expect(readDashboardOverviewSnapshotEntry("1d")).resolves.toBeNull();
    await expect(listDashboardOverviewSnapshotRanges()).resolves.toEqual(["usage"]);
  });
});

describe("fetchDashboardOverviewSnapshotBundle", () => {
  it("uses the fixed query matrix for each overview range", async () => {
    apiMocks.fetchDashboardActivity.mockResolvedValue({ range: "stub", summary: { stats: {} } });
    apiMocks.fetchDashboardNetworkTimeseries.mockResolvedValue({ range: "stub", points: [] });
    apiMocks.fetchParallelWorkStats.mockResolvedValue({
      current: {},
      minute7d: {},
      hour30d: {},
      dayAll: {},
    });
    apiMocks.fetchSummary.mockResolvedValue({ totalCount: 1 });
    apiMocks.fetchTimeseries.mockResolvedValue({
      rangeStart: "a",
      rangeEnd: "b",
      bucketSeconds: 60,
      points: [],
    });

    await fetchDashboardOverviewSnapshotBundle("today");
    await fetchDashboardOverviewSnapshotBundle("yesterday");
    await fetchDashboardOverviewSnapshotBundle("1d");
    await fetchDashboardOverviewSnapshotBundle("7d");
    await fetchDashboardOverviewSnapshotBundle("usage");

    expect(apiMocks.fetchDashboardActivity.mock.calls).toEqual([
      ["today", expect.objectContaining({ includeAccounts: false, includeRecent: false })],
      ["yesterday", expect.objectContaining({ includeAccounts: false, includeRecent: false })],
      ["1d", expect.objectContaining({ includeAccounts: false, includeRecent: false })],
      ["7d", expect.objectContaining({ includeAccounts: false, includeRecent: false })],
    ]);
    expect(apiMocks.fetchSummary.mock.calls).toEqual([
      ["yesterday", expect.any(Object)],
      ["previous7d", expect.any(Object)],
      ["previous7d", expect.any(Object)],
      ["1d", expect.any(Object)],
      ["7d", expect.any(Object)],
    ]);
    expect(apiMocks.fetchTimeseries.mock.calls).toEqual([
      ["today", expect.objectContaining({ bucket: "1m" })],
      ["yesterday", expect.objectContaining({ bucket: "1m" })],
      ["yesterday", expect.objectContaining({ bucket: "1m" })],
      ["1d", expect.objectContaining({ bucket: "1m" })],
      ["7d", expect.objectContaining({ bucket: "1h" })],
      ["6mo", expect.objectContaining({ bucket: "1d" })],
    ]);
    expect(apiMocks.fetchParallelWorkStats.mock.calls).toEqual([
      [expect.objectContaining({ range: "today", bucket: "1m" })],
      [expect.objectContaining({ range: "yesterday", bucket: "1m" })],
      [expect.objectContaining({ range: "yesterday", bucket: "1m" })],
    ]);
    expect(apiMocks.fetchDashboardNetworkTimeseries.mock.calls).toEqual([
      ["today", expect.any(Object)],
      ["yesterday", expect.any(Object)],
      ["1d", expect.any(Object)],
    ]);
  });
});
