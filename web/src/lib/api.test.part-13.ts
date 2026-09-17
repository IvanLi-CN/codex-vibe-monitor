import { expect, it, vi } from "vitest";
import { fetchSummary } from "./api";

it("forwards request signal to fetch for caller-managed cancellation", async () => {
  const fetchMock = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) => {
    void _input;
    void _init;
    return new Response(JSON.stringify({}), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  });
  vi.stubGlobal("fetch", fetchMock as typeof fetch);

  const controller = new AbortController();
  await fetchSummary("current", {
    timeZone: "UTC",
    signal: controller.signal,
  });

  expect(fetchMock).toHaveBeenCalledTimes(1);
  const firstCall = fetchMock.mock.calls[0];
  expect(firstCall).toBeDefined();
  const init = firstCall?.[1] as RequestInit | undefined;
  expect(init?.signal).toBe(controller.signal);
});
it("adds upstreamAccountId to summary query parameters", async () => {
  const fetchMock = vi.fn(async (_input: RequestInfo | URL) => {
    return new Response(
      JSON.stringify({
        totalCount: 0,
        successCount: 0,
        failureCount: 0,
        totalCost: 0,
        totalTokens: 0,
      }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    );
  });
  vi.stubGlobal("fetch", fetchMock as typeof fetch);

  await fetchSummary("today", {
    upstreamAccountId: 42,
  });

  const url = String(fetchMock.mock.calls[0]?.[0] ?? "");
  expect(url).toContain("window=today");
  expect(url).toContain("upstreamAccountId=42");
});
it("preserves optional maintenance stats fields", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      return new Response(
        JSON.stringify({
          totalCount: 5,
          successCount: 4,
          failureCount: 1,
          totalCost: 0.25,
          totalTokens: 128,
          maintenance: {
            rawCompressionBacklog: {
              oldestUncompressedAgeSecs: 90061,
              uncompressedCount: 3,
              uncompressedBytes: 2048,
              alertLevel: "warn",
            },
            startupBackfill: {
              upstreamActivityArchivePendingAccounts: 2,
              zeroUpdateStreak: 1,
              nextRunAfter: "2026-03-24T12:00:00Z",
            },
            historicalRollupBackfill: {
              pendingBuckets: 48,
              legacyArchivePending: 2,
              lastMaterializedHour: "2026-03-24T00:00:00Z",
              alertLevel: "warn",
            },
          },
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    }) as typeof fetch,
  );

  const response = await fetchSummary("1d", { timeZone: "UTC" });

  expect(response.maintenance).toEqual({
    rawCompressionBacklog: {
      oldestUncompressedAgeSecs: 90061,
      uncompressedCount: 3,
      uncompressedBytes: 2048,
      alertLevel: "warn",
    },
    startupBackfill: {
      upstreamActivityArchivePendingAccounts: 2,
      zeroUpdateStreak: 1,
      nextRunAfter: "2026-03-24T12:00:00Z",
    },
    historicalRollupBackfill: {
      pendingBuckets: 48,
      legacyArchivePending: 2,
      lastMaterializedHour: "2026-03-24T00:00:00Z",
      alertLevel: "warn",
    },
  });
});
