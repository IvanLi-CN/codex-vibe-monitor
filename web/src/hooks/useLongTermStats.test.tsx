/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LongTermStatsOverviewResponse } from "../lib/api";
import { LONG_TERM_STATS_REFRESH_INTERVAL_MS, useLongTermStats } from "./useLongTermStats";

const apiMocks = vi.hoisted(() => ({
  fetchOverview: vi.fn(),
  fetchSeries: vi.fn(),
}));

vi.mock("../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchLongTermStatsOverview: apiMocks.fetchOverview,
    fetchLongTermStatsSeries: apiMocks.fetchSeries,
  };
});

const overview: LongTermStatsOverviewResponse = {
  status: "ready",
  statisticsStartDate: "2026-01-01",
  processedRows: 3,
  totalRows: 3,
  timezone: "Asia/Shanghai",
  range: "7d",
  global: {
    calls: 3,
    tokens: 100,
    tokenSamples: 3,
    cost: 1,
    costSamples: 3,
    usageTimeMs: 100,
    usageTimeSamples: 3,
    wallTimeMs: 100,
    wallTimeSamples: 3,
    outputSpeedTokensPerSecond: 10,
    outputSpeedSamples: 3,
    firstByteMs: 20,
    firstByteSamples: 3,
    responseMs: 30,
    responseSamples: 3,
  },
  daily: [],
  models: [],
  upstreams: [],
};

let host: HTMLDivElement;
let root: Root;

function Probe() {
  const state = useLongTermStats("7d", "model", []);
  return <output data-testid="status">{state.overview?.status ?? "loading"}</output>;
}

beforeEach(() => {
  vi.useFakeTimers();
  apiMocks.fetchOverview.mockReset().mockResolvedValue(overview);
  apiMocks.fetchSeries.mockReset();
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.useRealTimers();
});

describe("useLongTermStats", () => {
  it("refreshes the overview every 60 seconds", async () => {
    await act(async () => root.render(<Probe />));
    expect(apiMocks.fetchOverview).toHaveBeenCalledTimes(1);

    await act(async () => {
      vi.advanceTimersByTime(LONG_TERM_STATS_REFRESH_INTERVAL_MS);
    });
    expect(apiMocks.fetchOverview).toHaveBeenCalledTimes(2);
  });
});
