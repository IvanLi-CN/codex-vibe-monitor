import { afterEach, describe, expect, it, vi } from "vitest";
import {
  DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS,
  createOauthMailboxSession,
  fetchInvocationRecords,
  fetchForwardProxyLiveStats,
  fetchForwardProxyBindingNodes,
  fetchForwardProxyTimeseries,
  fetchParallelWorkStats,
  fetchPromptCacheConversations,
  fetchTimeseries,
  fetchSettings,
  fetchSummary,
  fetchUpstreamAccountDetail,
  fetchUpstreamAccounts,
  fetchUpstreamStickyConversations,
  updateOauthLoginSession,
  updatePoolRoutingSettings,
  validateForwardProxyCandidate,
} from "./api";

function abortError(): Error {
  const error = new Error("aborted");
  error.name = "AbortError";
  return error;
}

function createAbortAwareFetch() {
  return vi.fn((_input: RequestInfo | URL, init?: RequestInit) => {
    return new Promise<Response>((_resolve, reject) => {
      const signal = init?.signal;
      if (!signal) return;
      if (signal.aborted) {
        reject(abortError());
        return;
      }
      signal.addEventListener(
        "abort",
        () => {
          reject(abortError());
        },
        { once: true },
      );
    });
  });
}

describe("validateForwardProxyCandidate timeout split", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("uses 60s timeout for subscription validation", async () => {
    vi.useFakeTimers();
    const fetchMock = createAbortAwareFetch();
    vi.stubGlobal("fetch", fetchMock as typeof fetch);

    const pending = validateForwardProxyCandidate({
      kind: "subscriptionUrl",
      value: "https://example.com/subscription",
    });

    const assertion = expect(pending).rejects.toThrow(
      "validation request timed out after 60s",
    );
    await vi.advanceTimersByTimeAsync(60_000);
    await assertion;
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("keeps 5s timeout for single proxy validation", async () => {
    vi.useFakeTimers();
    const fetchMock = createAbortAwareFetch();
    vi.stubGlobal("fetch", fetchMock as typeof fetch);

    const pending = validateForwardProxyCandidate({
      kind: "proxyUrl",
      value: "socks5://127.0.0.1:1080",
    });

    const assertion = expect(pending).rejects.toThrow(
      "validation request timed out after 5s",
    );
    await vi.advanceTimersByTimeAsync(5_000);
    await assertion;
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});

describe("fetchForwardProxyLiveStats", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("normalizes live proxy stats payload", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            rangeStart: "2026-03-01T00:00:00Z",
            rangeEnd: "2026-03-02T00:00:00Z",
            bucketSeconds: 3600,
            nodes: [
              {
                key: "__direct__",
                source: "direct",
                displayName: "Direct",
                weight: 1,
                penalized: false,
                stats: {
                  oneMinute: {
                    attempts: 2,
                    successRate: 0.5,
                    avgLatencyMs: 123,
                  },
                  fifteenMinutes: {
                    attempts: 10,
                    successRate: 0.6,
                    avgLatencyMs: 130,
                  },
                  oneHour: {
                    attempts: 40,
                    successRate: 0.7,
                    avgLatencyMs: 140,
                  },
                  oneDay: {
                    attempts: 200,
                    successRate: 0.8,
                    avgLatencyMs: 150,
                  },
                  sevenDays: {
                    attempts: 1200,
                    successRate: 0.9,
                    avgLatencyMs: 160,
                  },
                },
                last24h: [
                  {
                    bucketStart: "2026-03-01T00:00:00Z",
                    bucketEnd: "2026-03-01T01:00:00Z",
                    successCount: 3,
                    failureCount: 1,
                  },
                  {
                    bucketStart: "",
                    bucketEnd: "",
                    successCount: 99,
                    failureCount: 99,
                  },
                ],
                weight24h: [
                  {
                    bucketStart: "2026-03-01T00:00:00Z",
                    bucketEnd: "2026-03-01T01:00:00Z",
                    sampleCount: 3,
                    minWeight: 0.32,
                    maxWeight: 0.95,
                    avgWeight: 0.61,
                    lastWeight: 0.88,
                  },
                  {
                    bucketStart: "",
                    bucketEnd: "",
                    sampleCount: 99,
                    minWeight: 1,
                    maxWeight: 1,
                    avgWeight: 1,
                    lastWeight: 1,
                  },
                ],
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchForwardProxyLiveStats();
    expect(response.bucketSeconds).toBe(3600);
    expect(response.nodes).toHaveLength(1);
    expect(response.nodes[0].displayName).toBe("Direct");
    expect(response.nodes[0].stats.oneMinute.attempts).toBe(2);
    expect(response.nodes[0].last24h).toHaveLength(1);
    expect(response.nodes[0].last24h[0].successCount).toBe(3);
    expect(response.nodes[0].last24h[0].failureCount).toBe(1);
    expect(response.nodes[0].weight24h).toHaveLength(1);
    expect(response.nodes[0].weight24h[0].sampleCount).toBe(3);
    expect(response.nodes[0].weight24h[0].lastWeight).toBe(0.88);
  });

  it("falls back to empty weight buckets when backend payload omits weight24h", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            rangeStart: "2026-03-01T00:00:00Z",
            rangeEnd: "2026-03-02T00:00:00Z",
            bucketSeconds: 3600,
            nodes: [
              {
                key: "__direct__",
                source: "direct",
                displayName: "Direct",
                weight: 1,
                penalized: false,
                stats: {
                  oneMinute: { attempts: 0 },
                  fifteenMinutes: { attempts: 0 },
                  oneHour: { attempts: 0 },
                  oneDay: { attempts: 0 },
                  sevenDays: { attempts: 0 },
                },
                last24h: [],
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchForwardProxyLiveStats();
    expect(response.nodes).toHaveLength(1);
    expect(response.nodes[0].weight24h).toEqual([]);
  });
});

describe("fetchTimeseries", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("normalizes first-response-byte-total fields from the timeseries payload", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            rangeStart: "2026-03-26T12:00:00Z",
            rangeEnd: "2026-03-26T13:00:00Z",
            bucketSeconds: 900,
            effectiveBucket: "15m",
            availableBuckets: ["15m", "1h"],
            bucketLimitedToDaily: false,
            points: [
              {
                bucketStart: "2026-03-26T12:00:00Z",
                bucketEnd: "2026-03-26T12:15:00Z",
                totalCount: 11,
                successCount: 10,
                failureCount: 1,
                inFlightCount: 2,
                totalTokens: 193414,
                totalCost: 0.0543,
                firstByteSampleCount: 10,
                firstByteAvgMs: 81.7,
                firstByteP95Ms: 95.2,
                firstResponseByteTotalSampleCount: 10,
                firstResponseByteTotalAvgMs: 43890,
                firstResponseByteTotalP95Ms: 52340,
              },
              {
                bucketStart: "2026-03-26T12:15:00Z",
                bucketEnd: "2026-03-26T12:30:00Z",
                totalCount: 0,
                successCount: 0,
                failureCount: 0,
                inFlightCount: Number.NaN,
                totalTokens: 0,
                totalCost: 0,
                firstResponseByteTotalSampleCount: Number.NaN,
                firstResponseByteTotalAvgMs: "bad",
                firstResponseByteTotalP95Ms: null,
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchTimeseries("1h", { bucket: "15m" });
    expect(response.bucketSeconds).toBe(900);
    expect(response.points).toHaveLength(2);
    expect(response.points[0].inFlightCount).toBe(2);
    expect(response.points[0].firstResponseByteTotalSampleCount).toBe(10);
    expect(response.points[0].firstResponseByteTotalAvgMs).toBe(43890);
    expect(response.points[0].firstResponseByteTotalP95Ms).toBe(52340);
    expect(response.points[1].firstResponseByteTotalSampleCount).toBe(0);
    expect(response.points[1].firstResponseByteTotalAvgMs).toBeNull();
    expect(response.points[1].firstResponseByteTotalP95Ms).toBeNull();
    expect(response.points[1].inFlightCount).toBe(0);
  });
});

describe("fetchParallelWorkStats", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("normalizes the three fixed parallel-work windows", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            minute7d: {
              rangeStart: "2026-03-01T00:00:00Z",
              rangeEnd: "2026-03-08T00:00:00Z",
              bucketSeconds: 60,
              completeBucketCount: 10080,
              activeBucketCount: 4132,
              minCount: 0,
              maxCount: 18,
              avgCount: 4.67,
              points: [
                {
                  bucketStart: "2026-03-07T10:00:00Z",
                  bucketEnd: "2026-03-07T10:01:00Z",
                  parallelCount: 4,
                },
              ],
            },
            hour30d: {
              rangeStart: "2026-02-06T00:00:00Z",
              rangeEnd: "2026-03-08T00:00:00Z",
              bucketSeconds: 3600,
              completeBucketCount: 720,
              activeBucketCount: 321,
              minCount: 0,
              maxCount: 9,
              avgCount: 2.13,
              points: [
                {
                  bucketStart: "2026-03-07T00:00:00Z",
                  bucketEnd: "2026-03-07T01:00:00Z",
                  parallelCount: 2,
                },
              ],
            },
            dayAll: {
              rangeStart: "2026-03-08T00:00:00Z",
              rangeEnd: "2026-03-08T00:00:00Z",
              bucketSeconds: 86400,
              completeBucketCount: 0,
              activeBucketCount: 0,
              minCount: null,
              maxCount: null,
              avgCount: null,
              points: [],
            },
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchParallelWorkStats();
    expect(response.minute7d.points[0]?.parallelCount).toBe(4);
    expect(response.hour30d.avgCount).toBe(2.13);
    expect(response.dayAll.completeBucketCount).toBe(0);
    expect(response.dayAll.avgCount).toBeNull();
  });

  it("preserves the caller time zone for fixed sub-hour offsets", async () => {
    const fetchMock = vi.fn(async () => {
      return new Response(
        JSON.stringify({
          minute7d: {
            rangeStart: "2026-03-01T00:00:00Z",
            rangeEnd: "2026-03-08T00:00:00Z",
            bucketSeconds: 60,
            completeBucketCount: 1,
            activeBucketCount: 1,
            minCount: 1,
            maxCount: 1,
            avgCount: 1,
            points: [
              {
                bucketStart: "2026-03-07T10:00:00Z",
                bucketEnd: "2026-03-07T10:01:00Z",
                parallelCount: 1,
              },
            ],
          },
          hour30d: {
            rangeStart: "2026-03-01T00:00:00Z",
            rangeEnd: "2026-03-08T00:00:00Z",
            bucketSeconds: 3600,
            completeBucketCount: 1,
            activeBucketCount: 1,
            minCount: 1,
            maxCount: 1,
            avgCount: 1,
            points: [
              {
                bucketStart: "2026-03-07T10:00:00Z",
                bucketEnd: "2026-03-07T11:00:00Z",
                parallelCount: 1,
              },
            ],
          },
          dayAll: {
            rangeStart: "2026-03-01T00:00:00Z",
            rangeEnd: "2026-03-08T00:00:00Z",
            bucketSeconds: 86400,
            completeBucketCount: 1,
            activeBucketCount: 1,
            minCount: 1,
            maxCount: 1,
            avgCount: 1,
            points: [
              {
                bucketStart: "2026-03-07T00:00:00Z",
                bucketEnd: "2026-03-08T00:00:00Z",
                parallelCount: 1,
              },
            ],
          },
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    });
    vi.stubGlobal("fetch", fetchMock as typeof fetch);

    await fetchParallelWorkStats({ timeZone: "Asia/Kolkata" });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const firstArg = fetchMock.mock.calls.at(0)?.at(0) as
      | RequestInfo
      | URL
      | undefined;
    expect(firstArg).toBeDefined();
    expect(String(firstArg)).toBe(
      "/api/stats/parallel-work?timeZone=Asia%2FKolkata",
    );
  });

  it("preserves the caller time zone for seasonal sub-hour offsets", async () => {
    const fetchMock = vi.fn(async () => {
      return new Response(
        JSON.stringify({
          minute7d: {
            rangeStart: "2026-03-01T00:00:00Z",
            rangeEnd: "2026-03-08T00:00:00Z",
            bucketSeconds: 60,
            completeBucketCount: 0,
            activeBucketCount: 0,
            minCount: null,
            maxCount: null,
            avgCount: null,
            points: [],
          },
          hour30d: {
            rangeStart: "2026-03-01T00:00:00Z",
            rangeEnd: "2026-03-08T00:00:00Z",
            bucketSeconds: 3600,
            completeBucketCount: 0,
            activeBucketCount: 0,
            minCount: null,
            maxCount: null,
            avgCount: null,
            points: [],
          },
          dayAll: {
            rangeStart: "2026-03-01T00:00:00Z",
            rangeEnd: "2026-03-08T00:00:00Z",
            bucketSeconds: 86400,
            completeBucketCount: 0,
            activeBucketCount: 0,
            minCount: null,
            maxCount: null,
            avgCount: null,
            points: [],
          },
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    });
    vi.stubGlobal("fetch", fetchMock as typeof fetch);

    await fetchParallelWorkStats({ timeZone: "Australia/Lord_Howe" });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const firstArg = fetchMock.mock.calls.at(0)?.at(0) as
      | RequestInfo
      | URL
      | undefined;
    expect(firstArg).toBeDefined();
    expect(String(firstArg)).toBe(
      "/api/stats/parallel-work?timeZone=Australia%2FLord_Howe",
    );
  });
});

describe("updateOauthLoginSession", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("preserves syncApplied=false from stale session sync responses", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            loginId: "login-1",
            status: "pending",
            authUrl: "https://auth.openai.com/authorize?login=1",
            redirectUri: "http://localhost:1455/oauth/callback",
            expiresAt: "2026-03-13T10:00:00.000Z",
            updatedAt: "2026-03-13T10:01:00.000Z",
            accountId: null,
            error: null,
            syncApplied: false,
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await updateOauthLoginSession("login-1", {
      displayName: "Fresh OAuth",
      tagIds: [],
      isMother: false,
    });

    expect(response.syncApplied).toBe(false);
    expect(response.updatedAt).toBe("2026-03-13T10:01:00.000Z");
  });
});

describe("fetchForwardProxyTimeseries", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("normalizes historical proxy timeseries payload", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            rangeStart: "2026-01-01T00:00:00Z",
            rangeEnd: "2026-01-08T00:00:00Z",
            bucketSeconds: 3600,
            effectiveBucket: "1h",
            availableBuckets: ["1h"],
            nodes: [
              {
                key: "__archived__",
                source: "archived",
                displayName: "Archived Proxy",
                weight: 0.8,
                penalized: false,
                buckets: [
                  {
                    bucketStart: "2026-01-01T00:00:00Z",
                    bucketEnd: "2026-01-01T01:00:00Z",
                    successCount: 4,
                    failureCount: 1,
                  },
                  {
                    bucketStart: "",
                    bucketEnd: "",
                    successCount: 99,
                    failureCount: 99,
                  },
                ],
                weightBuckets: [
                  {
                    bucketStart: "2026-01-01T00:00:00Z",
                    bucketEnd: "2026-01-01T01:00:00Z",
                    sampleCount: 2,
                    minWeight: 0.5,
                    maxWeight: 0.9,
                    avgWeight: 0.7,
                    lastWeight: 0.8,
                  },
                  {
                    bucketStart: "",
                    bucketEnd: "",
                    sampleCount: 99,
                    minWeight: 1,
                    maxWeight: 1,
                    avgWeight: 1,
                    lastWeight: 1,
                  },
                ],
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchForwardProxyTimeseries("7d", {
      bucket: "1h",
      timeZone: "UTC",
    });

    expect(response.rangeStart).toBe("2026-01-01T00:00:00Z");
    expect(response.effectiveBucket).toBe("1h");
    expect(response.availableBuckets).toEqual(["1h"]);
    expect(response.nodes).toHaveLength(1);
    expect(response.nodes[0].displayName).toBe("Archived Proxy");
    expect(response.nodes[0].buckets).toHaveLength(1);
    expect(response.nodes[0].buckets[0].successCount).toBe(4);
    expect(response.nodes[0].weightBuckets).toHaveLength(1);
    expect(response.nodes[0].weightBuckets[0].lastWeight).toBe(0.8);
  });

  it("rejects non-whole-hour proxy history time zones instead of rewriting them", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock as typeof fetch);

    await expect(
      fetchForwardProxyTimeseries("today", {
        bucket: "1h",
        timeZone: "Asia/Kolkata",
      }),
    ).rejects.toThrow("whole-hour UTC offsets");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("rejects seasonal half-hour proxy history time zones when the queried range crosses them", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-10-10T00:00:00Z"));
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock as typeof fetch);

    await expect(
      fetchForwardProxyTimeseries("1mo", {
        bucket: "1h",
        timeZone: "Australia/Lord_Howe",
      }),
    ).rejects.toThrow("whole-hour UTC offsets");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("rejects short proxy history ranges that cross a sub-hour DST transition", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-10-03T15:40:00Z"));
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock as typeof fetch);

    await expect(
      fetchForwardProxyTimeseries("30m", {
        bucket: "1h",
        timeZone: "Australia/Lord_Howe",
      }),
    ).rejects.toThrow("whole-hour UTC offsets");
    expect(fetchMock).not.toHaveBeenCalled();
  });
});

describe("fetchSummary", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("forwards request signal to fetch for caller-managed cancellation", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) => {
        void _input;
        void _init;
        return new Response(JSON.stringify({}), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        });
      },
    );
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
});

describe("createOauthMailboxSession", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("accepts unsupported responses even when emailAddress is blank", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            supported: false,
            emailAddress: "",
            reason: "invalid_format",
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    await expect(
      createOauthMailboxSession({ emailAddress: "" }),
    ).resolves.toEqual({
      supported: false,
      emailAddress: "",
      reason: "invalid_format",
    });
  });
});

describe("settings normalization", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("normalizes forward proxy settings when fetching settings", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            forwardProxy: {
              proxyUrls: ["socks5://127.0.0.1:1080"],
              subscriptionUrls: ["https://example.com/subscription.txt"],
              subscriptionUpdateIntervalSecs: 900,
              nodes: [
                {
                  key: "fpb_jp_edge_01",
                  source: "manual",
                  displayName: "JP Edge 01",
                  endpointUrl: "socks5://127.0.0.1:1080",
                  weight: 0.9,
                  penalized: false,
                  stats: {
                    oneMinute: { attempts: 2 },
                    fifteenMinutes: { attempts: 10 },
                    oneHour: { attempts: 20 },
                    oneDay: { attempts: 30 },
                    sevenDays: { attempts: 40 },
                  },
                },
              ],
            },
            pricing: {
              catalogVersion: "v1",
              entries: [],
            },
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const settings = await fetchSettings();
    expect(settings.forwardProxy.subscriptionUpdateIntervalSecs).toBe(900);
    expect(settings.forwardProxy.nodes).toHaveLength(1);
    expect(settings.forwardProxy.nodes[0].displayName).toBe("JP Edge 01");
  });

  it("normalizes bound proxy keys and binding nodes in upstream account list", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            writesEnabled: true,
            items: [],
            groups: [
              {
                groupName: "production",
                note: "Premium traffic",
                boundProxyKeys: [
                  "fpb_jp_edge_01",
                  "fpb_sg_edge_02",
                  "fpb_jp_edge_01",
                ],
                upstream429RetryEnabled: true,
                upstream429MaxRetries: 3,
              },
            ],
            forwardProxyNodes: [
              {
                key: "fpb_jp_edge_01",
                aliasKeys: ["fpn_jp_edge_runtime"],
                source: "manual",
                displayName: "JP Edge 01",
                protocolLabel: "HTTP",
                penalized: false,
                selectable: true,
                last24h: [
                  {
                    bucketStart: "2026-03-01T00:00:00Z",
                    bucketEnd: "2026-03-01T01:00:00Z",
                    successCount: 5,
                    failureCount: 1,
                  },
                ],
              },
              {
                key: "fpn_deadbeefcafebabe",
                source: "missing",
                displayName: "历史东京中继",
                protocolLabel: "unknown",
                penalized: false,
                selectable: false,
                last24h: [],
              },
            ],
            hasUngroupedAccounts: false,
            total: 0,
            page: 1,
            pageSize: 20,
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchUpstreamAccounts();
    expect(response.groups[0].boundProxyKeys).toEqual([
      "fpb_jp_edge_01",
      "fpb_sg_edge_02",
      "fpb_jp_edge_01",
    ]);
    expect(response.groups[0].upstream429RetryEnabled).toBe(true);
    expect(response.groups[0].upstream429MaxRetries).toBe(3);
    expect(response.forwardProxyNodes ?? []).toHaveLength(2);
    expect(response.forwardProxyNodes?.[0]?.protocolLabel).toBe("HTTP");
    expect(response.forwardProxyNodes?.[0]?.aliasKeys).toEqual([
      "fpn_jp_edge_runtime",
    ]);
    expect(response.forwardProxyNodes?.[1]?.selectable).toBe(false);
    expect(response.forwardProxyNodes?.[1]?.protocolLabel).toBe("UNKNOWN");
    expect(response.forwardProxyNodes?.[1]?.displayName).toBe("历史东京中继");
    expect(response.forwardProxyNodes?.[0]?.last24h[0]?.successCount).toBe(5);
    expect(response.forwardProxyNodes?.[1]?.last24h).toEqual([]);
  });

  it("fetches forward proxy binding nodes through the dedicated endpoint", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      expect(String(input)).toContain(
        "/api/pool/forward-proxy-binding-nodes?includeCurrent=1&key=fpb_jp_edge_01&key=fpb_sg_edge_02",
      );
      return new Response(
        JSON.stringify([
          {
            key: "fpb_jp_edge_01",
            aliasKeys: ["fpn_jp_edge_runtime"],
            source: "manual",
            displayName: "JP Edge 01",
            protocolLabel: "HTTP",
            penalized: false,
            selectable: true,
            last24h: [],
          },
        ]),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    });
    vi.stubGlobal("fetch", fetchMock as typeof fetch);

    const response = await fetchForwardProxyBindingNodes([
      "fpb_jp_edge_01",
      " ",
      "fpb_sg_edge_02",
      "fpb_jp_edge_01",
    ], {
      includeCurrent: true,
    });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(response).toHaveLength(1);
    expect(response[0]).toMatchObject({
      key: "fpb_jp_edge_01",
      protocolLabel: "HTTP",
      aliasKeys: ["fpn_jp_edge_runtime"],
    });
  });

  it("includes groupName when requesting group-scoped binding node stats", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      expect(String(input)).toContain(
        "/api/pool/forward-proxy-binding-nodes?includeCurrent=1&groupName=prod&key=fpb_jp_edge_01",
      );
      return new Response(JSON.stringify([]), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    });
    vi.stubGlobal("fetch", fetchMock as typeof fetch);

    await fetchForwardProxyBindingNodes(["fpb_jp_edge_01"], {
      includeCurrent: true,
      groupName: "  prod  ",
    });

    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});

describe("account pool frontend API helpers", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("adds requestId to invocation records query parameters", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      expect(url).toContain("/api/invocations?");
      expect(url).toContain("requestId=invoke-123");
      expect(url).not.toContain("proxy=");
      return new Response(
        JSON.stringify({
          snapshotId: 1,
          total: 0,
          page: 1,
          pageSize: 20,
          records: [],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    });
    vi.stubGlobal("fetch", fetchMock as typeof fetch);

    await fetchInvocationRecords({ requestId: "invoke-123" });

    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("adds stickyKey and upstreamAccountId to invocation records query parameters", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      expect(url).toContain("/api/invocations?");
      expect(url).toContain("stickyKey=sticky-001");
      expect(url).toContain("upstreamAccountId=42");
      return new Response(
        JSON.stringify({
          snapshotId: 7,
          total: 0,
          page: 1,
          pageSize: 20,
          records: [],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    });
    vi.stubGlobal("fetch", fetchMock as typeof fetch);

    await fetchInvocationRecords({
      stickyKey: "sticky-001",
      upstreamAccountId: 42,
    });

    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("normalizes routing settings from the upstream account list payload", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            writesEnabled: true,
            groups: [],
            hasUngroupedAccounts: false,
            routing: {
              apiKeyConfigured: true,
              maskedApiKey: "pool-live••••••c0de",
            },
            items: [],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchUpstreamAccounts();

    expect(response.routing).toEqual({
      writesEnabled: true,
      apiKeyConfigured: true,
      maskedApiKey: "pool-live••••••c0de",
      maintenance: {
        primarySyncIntervalSecs:
          DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS.primarySyncIntervalSecs,
        secondarySyncIntervalSecs:
          DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS.secondarySyncIntervalSecs,
        priorityAvailableAccountCap:
          DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS.priorityAvailableAccountCap,
      },
      timeouts: {
        responsesFirstByteTimeoutSecs: 120,
        compactFirstByteTimeoutSecs: 300,
        responsesStreamTimeoutSecs: 300,
        compactStreamTimeoutSecs: 300,
      },
    });
  });

  it("normalizes explicit routing timeouts from the upstream account list payload", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            writesEnabled: true,
            groups: [],
            hasUngroupedAccounts: false,
            routing: {
              apiKeyConfigured: true,
              maskedApiKey: "pool-live••••••c0de",
              timeouts: {
                responsesFirstByteTimeoutSecs: 180,
                compactFirstByteTimeoutSecs: 420,
                responsesStreamTimeoutSecs: 360,
                compactStreamTimeoutSecs: 540,
              },
            },
            items: [],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchUpstreamAccounts();

    expect(response.routing?.timeouts).toEqual({
      responsesFirstByteTimeoutSecs: 180,
      compactFirstByteTimeoutSecs: 420,
      responsesStreamTimeoutSecs: 360,
      compactStreamTimeoutSecs: 540,
    });
  });

  it("normalizes duplicate info from upstream account payloads", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            writesEnabled: true,
            groups: [],
            hasUngroupedAccounts: false,
            items: [
              {
                id: 1,
                kind: "oauth_codex",
                provider: "codex",
                displayName: "Dup OAuth",
                isMother: false,
                status: "active",
                enabled: true,
                duplicateInfo: {
                  peerAccountIds: [2],
                  reasons: ["sharedChatgptAccountId"],
                },
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchUpstreamAccounts();

    expect(response.items[0]?.duplicateInfo).toEqual({
      peerAccountIds: [2],
      reasons: ["sharedChatgptAccountId"],
    });
  });

  it("normalizes compact support state from upstream account payloads", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            writesEnabled: true,
            groups: [],
            hasUngroupedAccounts: false,
            items: [
              {
                id: 1,
                kind: "oauth_codex",
                provider: "codex",
                displayName: "Compact Probe",
                isMother: false,
                status: "active",
                enabled: true,
                compactSupport: {
                  status: "unsupported",
                  observedAt: "2026-03-16T02:08:00.000Z",
                  reason:
                    "No available channel for compact model gpt-5.4-openai-compact",
                },
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchUpstreamAccounts();

    expect(response.items[0]?.compactSupport).toEqual({
      status: "unsupported",
      observedAt: "2026-03-16T02:08:00.000Z",
      reason: "No available channel for compact model gpt-5.4-openai-compact",
    });
  });

  it("normalizes active conversation counts from upstream account payloads", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            writesEnabled: true,
            groups: [],
            hasUngroupedAccounts: false,
            items: [
              {
                id: 1,
                kind: "oauth_codex",
                provider: "codex",
                displayName: "Working OAuth",
                isMother: false,
                status: "active",
                enabled: true,
                activeConversationCount: 4,
              },
              {
                id: 2,
                kind: "api_key_codex",
                provider: "codex",
                displayName: "Missing Count API key",
                isMother: false,
                status: "active",
                enabled: true,
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchUpstreamAccounts();

    expect(response.items[0]?.activeConversationCount).toBe(4);
    expect(response.items[1]?.activeConversationCount).toBe(0);
  });

  it("normalizes active conversation counts from upstream account detail payloads", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            id: 9,
            kind: "oauth_codex",
            provider: "codex",
            displayName: "Detail OAuth",
            isMother: false,
            status: "active",
            enabled: true,
            activeConversationCount: 2,
            history: [],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchUpstreamAccountDetail(9);

    expect(response.activeConversationCount).toBe(2);
  });

  it("normalizes tag fast mode values from upstream account roster payloads", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            writesEnabled: true,
            groups: [],
            hasUngroupedAccounts: false,
            items: [
              {
                id: 1,
                kind: "oauth_codex",
                provider: "codex",
                displayName: "Fast Mode OAuth",
                isMother: false,
                status: "active",
                enabled: true,
                tags: [
                  {
                    id: 31,
                    name: "priority-route",
                    routingRule: {
                      guardEnabled: false,
                      allowCutOut: true,
                      allowCutIn: true,
                      priorityTier: "primary",
                      fastModeRewriteMode: "force_add",
                    },
                  },
                ],
                effectiveRoutingRule: {
                  guardEnabled: false,
                  allowCutOut: true,
                  allowCutIn: true,
                  priorityTier: "fallback",
                  fastModeRewriteMode: "force_remove",
                  sourceTagIds: [31],
                  sourceTagNames: ["priority-route"],
                  guardRules: [],
                },
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchUpstreamAccounts();

    expect(response.items[0]?.tags?.[0]?.routingRule.fastModeRewriteMode).toBe(
      "force_add",
    );
    expect(response.items[0]?.effectiveRoutingRule?.fastModeRewriteMode).toBe(
      "force_remove",
    );
  });

  it("falls back to keep_original when upstream account detail fast mode is missing or invalid", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            id: 9,
            kind: "oauth_codex",
            provider: "codex",
            displayName: "Detail OAuth",
            isMother: false,
            status: "active",
            enabled: true,
            tags: [
              {
                id: 41,
                name: "legacy-tag",
                routingRule: {
                  guardEnabled: false,
                  allowCutOut: true,
                  allowCutIn: true,
                  priorityTier: "normal",
                },
              },
            ],
            effectiveRoutingRule: {
              guardEnabled: false,
              allowCutOut: true,
              allowCutIn: true,
              priorityTier: "normal",
              fastModeRewriteMode: "unexpected",
              sourceTagIds: [41],
              sourceTagNames: ["legacy-tag"],
              guardRules: [],
            },
            history: [],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchUpstreamAccountDetail(9);

    expect(response.tags[0]?.routingRule.fastModeRewriteMode).toBe(
      "keep_original",
    );
    expect(response.effectiveRoutingRule?.fastModeRewriteMode).toBe(
      "keep_original",
    );
  });

  it("normalizes window actual usage from upstream account roster payloads", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            writesEnabled: true,
            groups: [],
            hasUngroupedAccounts: false,
            items: [
              {
                id: 1,
                kind: "oauth_codex",
                provider: "codex",
                displayName: "Usage OAuth",
                isMother: false,
                status: "active",
                enabled: true,
                primaryWindow: {
                  usedPercent: 42,
                  usedText: "42% used",
                  limitText: "5h rolling window",
                  resetsAt: "2026-03-29T14:27:00.000Z",
                  windowDurationMins: 300,
                  actualUsage: {
                    requestCount: 17,
                    totalTokens: 48210,
                    totalCost: 0.4284,
                    inputTokens: 28140,
                    outputTokens: 16410,
                    cacheInputTokens: 3660,
                  },
                },
                secondaryWindow: {
                  usedPercent: 18,
                  usedText: "18% used",
                  limitText: "7d rolling window",
                  resetsAt: "2026-04-05T14:27:00.000Z",
                  windowDurationMins: 10080,
                  actualUsage: {
                    requestCount: 73,
                    totalTokens: 182340,
                    totalCost: 1.6234,
                    inputTokens: 103220,
                    outputTokens: 67480,
                    cacheInputTokens: 11640,
                  },
                },
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchUpstreamAccounts();

    expect(response.items[0]?.primaryWindow?.actualUsage).toEqual({
      requestCount: 17,
      totalTokens: 48210,
      totalCost: 0.4284,
      inputTokens: 28140,
      outputTokens: 16410,
      cacheInputTokens: 3660,
    });
    expect(response.items[0]?.secondaryWindow?.actualUsage).toEqual({
      requestCount: 73,
      totalTokens: 182340,
      totalCost: 1.6234,
      inputTokens: 103220,
      outputTokens: 67480,
      cacheInputTokens: 11640,
    });
  });

  it("normalizes window actual usage from upstream account detail payloads", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            id: 9,
            kind: "oauth_codex",
            provider: "codex",
            displayName: "Detail OAuth",
            isMother: false,
            status: "active",
            enabled: true,
            primaryWindow: {
              usedPercent: 9,
              usedText: "9% used",
              limitText: "5h rolling window",
              resetsAt: "2026-03-29T14:27:00.000Z",
              windowDurationMins: 300,
              actualUsage: {
                requestCount: 4,
                totalTokens: 12144,
                totalCost: 0.1042,
                inputTokens: 7056,
                outputTokens: 4032,
                cacheInputTokens: 1056,
              },
            },
            history: [],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchUpstreamAccountDetail(9);

    expect(response.primaryWindow?.actualUsage).toEqual({
      requestCount: 4,
      totalTokens: 12144,
      totalCost: 0.1042,
      inputTokens: 7056,
      outputTokens: 4032,
      cacheInputTokens: 1056,
    });
  });

  it("serializes upstream account roster filters into the query string", async () => {
    const fetchMock = vi.fn(async (_input: RequestInfo | URL) => {
      expect(String(_input)).toContain(
        "/api/pool/upstream-accounts?groupSearch=prod&groupUngrouped=false&workStatus=degraded&workStatus=rate_limited&workStatus=working&enableStatus=enabled&healthStatus=normal&healthStatus=needs_reauth&tagIds=1&tagIds=2",
      );
      return new Response(
        JSON.stringify({
          writesEnabled: true,
          groups: [],
          hasUngroupedAccounts: true,
          items: [],
          routing: {
            apiKeyConfigured: false,
            maskedApiKey: null,
          },
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    });
    vi.stubGlobal("fetch", fetchMock as typeof fetch);

    const response = await fetchUpstreamAccounts({
      groupSearch: "prod",
      groupUngrouped: false,
      workStatus: ["degraded", "rate_limited", "working"],
      enableStatus: ["enabled"],
      healthStatus: ["normal", "needs_reauth"],
      tagIds: [1, 2],
    });

    expect(response.hasUngroupedAccounts).toBe(true);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("normalizes split status dimensions from legacy upstream account payloads", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            writesEnabled: true,
            groups: [],
            hasUngroupedAccounts: false,
            items: [
              {
                id: 9,
                kind: "oauth_codex",
                provider: "codex",
                displayName: "Legacy OAuth",
                isMother: false,
                status: "syncing",
                displayStatus: "needs_reauth",
                enabled: true,
              },
              {
                id: 10,
                kind: "api_key_codex",
                provider: "codex",
                displayName: "Legacy API key",
                isMother: false,
                status: "disabled",
                displayStatus: "disabled",
                enabled: false,
              },
              {
                id: 11,
                kind: "oauth_codex",
                provider: "codex",
                displayName: "Legacy upstream rejected",
                isMother: false,
                status: "error",
                displayStatus: "upstream_rejected",
                enabled: true,
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchUpstreamAccounts();

    expect(response.items[0]).toMatchObject({
      enableStatus: "enabled",
      workStatus: "idle",
      healthStatus: "needs_reauth",
      syncState: "syncing",
    });
    expect(response.items[1]).toMatchObject({
      enableStatus: "disabled",
      workStatus: "idle",
      healthStatus: "normal",
      syncState: "idle",
    });
    expect(response.items[2]).toMatchObject({
      enableStatus: "enabled",
      workStatus: "unavailable",
      healthStatus: "upstream_rejected",
      syncState: "idle",
    });
  });

  it("normalizes current forward proxy fields from upstream account payloads", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            writesEnabled: true,
            groups: [],
            hasUngroupedAccounts: false,
            items: [
              {
                id: 21,
                kind: "oauth_codex",
                provider: "codex",
                displayName: "Assigned OAuth",
                isMother: false,
                status: "active",
                enabled: true,
                currentForwardProxyKey: "jp-edge-01",
                currentForwardProxyDisplayName: "JP Edge 01",
                currentForwardProxyState: "assigned",
              },
              {
                id: 22,
                kind: "api_key_codex",
                provider: "codex",
                displayName: "Pending API Key",
                isMother: false,
                status: "active",
                enabled: true,
                currentForwardProxyState: "pending",
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchUpstreamAccounts();

    expect(response.items[0]).toMatchObject({
      currentForwardProxyKey: "jp-edge-01",
      currentForwardProxyDisplayName: "JP Edge 01",
      currentForwardProxyState: "assigned",
    });
    expect(response.items[1]).toMatchObject({
      currentForwardProxyKey: null,
      currentForwardProxyDisplayName: null,
      currentForwardProxyState: "pending",
    });
  });

  it("saves pool routing settings through the dedicated endpoint", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, init?: RequestInit) => {
        expect(String(_input)).toContain("/api/pool/routing-settings");
        expect(init?.method).toBe("PUT");
        expect(JSON.parse(String(init?.body))).toEqual({
          apiKey: "pool-secret",
          timeouts: {
            responsesFirstByteTimeoutSecs: 180,
            compactFirstByteTimeoutSecs: 420,
            responsesStreamTimeoutSecs: 360,
            compactStreamTimeoutSecs: 540,
          },
        });
        return new Response(
          JSON.stringify({
            apiKeyConfigured: true,
            maskedApiKey: "pool-live••••••cret",
            maintenance: {
              primarySyncIntervalSecs: 300,
              secondarySyncIntervalSecs: 1800,
              priorityAvailableAccountCap: 100,
            },
            timeouts: {
              responsesFirstByteTimeoutSecs: 180,
              compactFirstByteTimeoutSecs: 420,
              responsesStreamTimeoutSecs: 360,
              compactStreamTimeoutSecs: 540,
            },
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      },
    );
    vi.stubGlobal("fetch", fetchMock as typeof fetch);

    const response = await updatePoolRoutingSettings({
      apiKey: "pool-secret",
      timeouts: {
        responsesFirstByteTimeoutSecs: 180,
        compactFirstByteTimeoutSecs: 420,
        responsesStreamTimeoutSecs: 360,
        compactStreamTimeoutSecs: 540,
      },
    });

    expect(response.apiKeyConfigured).toBe(true);
    expect(response.writesEnabled).toBe(true);
    expect(response.maskedApiKey).toBe("pool-live••••••cret");
    expect(response.maintenance).toEqual({
      primarySyncIntervalSecs: 300,
      secondarySyncIntervalSecs: 1800,
      priorityAvailableAccountCap: 100,
    });
    expect(response.timeouts).toEqual({
      responsesFirstByteTimeoutSecs: 180,
      compactFirstByteTimeoutSecs: 420,
      responsesStreamTimeoutSecs: 360,
      compactStreamTimeoutSecs: 540,
    });
  });

  it("normalizes sticky key conversations for one upstream account", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        expect(String(input)).toContain(
          "/api/pool/upstream-accounts/101/sticky-keys?limit=20",
        );
        return new Response(
          JSON.stringify({
            rangeStart: "2026-03-10T00:00:00Z",
            rangeEnd: "2026-03-11T00:00:00Z",
            selectionMode: "count",
            selectedLimit: 20,
            selectedActivityHours: null,
            implicitFilter: {
              kind: null,
              filteredCount: 0,
            },
            conversations: [
              {
                stickyKey: "sticky-001",
                requestCount: 2,
                totalTokens: 30,
                totalCost: 0.12,
                createdAt: "2026-03-10T01:00:00Z",
                lastActivityAt: "2026-03-10T02:00:00Z",
                recentInvocations: [
                  {
                    id: 101,
                    invokeId: "sticky-invoke-101",
                    occurredAt: "2026-03-10T02:00:00Z",
                    status: "completed",
                    failureClass: "none",
                    routeMode: "sticky",
                    model: "gpt-5.4",
                    totalTokens: 30,
                    cost: 0.12,
                    proxyDisplayName: "Proxy Alpha",
                    upstreamAccountId: 101,
                    upstreamAccountName: "Pool Alpha",
                    endpoint: "/v1/responses",
                    source: "proxy",
                    inputTokens: 18,
                    outputTokens: 12,
                    cacheInputTokens: 4,
                    reasoningTokens: 2,
                    reasoningEffort: "high",
                    responseContentEncoding: "br",
                    requestedServiceTier: "flex",
                    serviceTier: "scale",
                    tReqReadMs: 10,
                    tReqParseMs: 11,
                    tUpstreamConnectMs: 12,
                    tUpstreamTtfbMs: 13,
                    tUpstreamStreamMs: 14,
                    tRespParseMs: 15,
                    tPersistMs: 16,
                    tTotalMs: 91,
                  },
                ],
                last24hRequests: [
                  {
                    occurredAt: "2026-03-10T02:00:00Z",
                    status: "success",
                    isSuccess: true,
                    outcome: "success",
                    requestTokens: 30,
                    cumulativeTokens: 30,
                  },
                ],
              },
            ],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }) as typeof fetch,
    );

    const response = await fetchUpstreamStickyConversations(101, {
      mode: "count",
      limit: 20,
    });

    expect(response.selectionMode).toBe("count");
    expect(response.selectedLimit).toBe(20);
    expect(response.selectedActivityHours).toBeNull();
    expect(response.conversations).toHaveLength(1);
    expect(response.conversations[0]?.stickyKey).toBe("sticky-001");
    expect(response.conversations[0]?.recentInvocations[0]?.invokeId).toBe(
      "sticky-invoke-101",
    );
    expect(
      response.conversations[0]?.last24hRequests[0]?.cumulativeTokens,
    ).toBe(30);
    expect(response.conversations[0]?.last24hRequests[0]?.outcome).toBe(
      "success",
    );
  });

  it("sends activity-window prompt cache conversation queries and normalizes metadata", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      expect(String(input)).toContain(
        "/api/stats/prompt-cache-conversations?activityHours=3",
      );
      return new Response(
        JSON.stringify({
          rangeStart: "2026-03-10T21:00:00Z",
          rangeEnd: "2026-03-11T00:00:00Z",
          selectionMode: "activityWindow",
          selectedLimit: null,
          selectedActivityHours: 3,
          implicitFilter: {
            kind: "cappedTo50",
            filteredCount: 7,
          },
          conversations: [
            {
              promptCacheKey: "pck-001",
              requestCount: 2,
              totalTokens: 30,
              totalCost: 0.12,
              createdAt: "2026-03-10T22:00:00Z",
              lastActivityAt: "2026-03-10T23:00:00Z",
              upstreamAccounts: [
                {
                  upstreamAccountId: 42,
                  upstreamAccountName: "Pool Alpha",
                  requestCount: 2,
                  totalTokens: 30,
                  totalCost: 0.12,
                  lastActivityAt: "2026-03-10T23:00:00Z",
                },
              ],
              recentInvocations: [
                {
                  id: 17,
                  invokeId: "invoke-17",
                  occurredAt: "2026-03-10T23:00:00Z",
                  status: "completed",
                  failureClass: "none",
                  routeMode: "pool",
                  model: "gpt-5.4",
                  totalTokens: 30,
                  cost: 0.12,
                  proxyDisplayName: "Proxy Alpha",
                  upstreamAccountId: 42,
                  upstreamAccountName: "Pool Alpha",
                  endpoint: "/v1/responses",
                  source: "proxy",
                  inputTokens: 18,
                  outputTokens: 12,
                  cacheInputTokens: 6,
                  reasoningTokens: 3,
                  reasoningEffort: "high",
                  errorMessage: "[upstream_response_failed] preview error",
                  downstreamStatusCode: 502,
                  downstreamErrorMessage:
                    "pool upstream responded with 502: failed to contact oauth codex upstream",
                  failureKind: "upstream_response_failed",
                  isActionable: true,
                  responseContentEncoding: "br",
                  requestedServiceTier: "flex",
                  serviceTier: "scale",
                  tReqReadMs: 10,
                  tReqParseMs: 11,
                  tUpstreamConnectMs: 12,
                  tUpstreamTtfbMs: 13,
                  tUpstreamStreamMs: 14,
                  tRespParseMs: 15,
                  tPersistMs: 16,
                  tTotalMs: 91,
                },
              ],
              last24hRequests: [],
            },
          ],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    });
    vi.stubGlobal("fetch", fetchMock as typeof fetch);

    const response = await fetchPromptCacheConversations({
      mode: "activityWindow",
      activityHours: 3,
    });

    expect(response.selectionMode).toBe("activityWindow");
    expect(response.selectedActivityHours).toBe(3);
    expect(response.implicitFilter.kind).toBe("cappedTo50");
    expect(response.implicitFilter.filteredCount).toBe(7);
    expect(response.conversations[0]?.promptCacheKey).toBe("pck-001");
    expect(
      response.conversations[0]?.upstreamAccounts[0]?.upstreamAccountName,
    ).toBe("Pool Alpha");
    expect(response.conversations[0]?.recentInvocations[0]?.invokeId).toBe(
      "invoke-17",
    );
    expect(response.conversations[0]?.recentInvocations[0]?.failureClass).toBe(
      "none",
    );
    expect(response.conversations[0]?.recentInvocations[0]?.routeMode).toBe(
      "pool",
    );
    expect(response.conversations[0]?.recentInvocations[0]?.endpoint).toBe(
      "/v1/responses",
    );
    expect(response.conversations[0]?.recentInvocations[0]?.source).toBe(
      "proxy",
    );
    expect(response.conversations[0]?.recentInvocations[0]?.inputTokens).toBe(
      18,
    );
    expect(response.conversations[0]?.recentInvocations[0]?.outputTokens).toBe(
      12,
    );
    expect(
      response.conversations[0]?.recentInvocations[0]?.cacheInputTokens,
    ).toBe(6);
    expect(
      response.conversations[0]?.recentInvocations[0]?.reasoningTokens,
    ).toBe(3);
    expect(
      response.conversations[0]?.recentInvocations[0]?.reasoningEffort,
    ).toBe("high");
    expect(response.conversations[0]?.recentInvocations[0]?.failureKind).toBe(
      "upstream_response_failed",
    );
    expect(
      response.conversations[0]?.recentInvocations[0]?.downstreamStatusCode,
    ).toBe(502);
    expect(
      response.conversations[0]?.recentInvocations[0]?.downstreamErrorMessage,
    ).toBe(
      "pool upstream responded with 502: failed to contact oauth codex upstream",
    );
    expect(response.conversations[0]?.recentInvocations[0]?.isActionable).toBe(
      true,
    );
    expect(
      response.conversations[0]?.recentInvocations[0]?.responseContentEncoding,
    ).toBe("br");
    expect(
      response.conversations[0]?.recentInvocations[0]?.requestedServiceTier,
    ).toBe("flex");
    expect(response.conversations[0]?.recentInvocations[0]?.serviceTier).toBe(
      "scale",
    );
    expect(response.conversations[0]?.recentInvocations[0]?.tReqReadMs).toBe(
      10,
    );
    expect(response.conversations[0]?.recentInvocations[0]?.tReqParseMs).toBe(
      11,
    );
    expect(
      response.conversations[0]?.recentInvocations[0]?.tUpstreamConnectMs,
    ).toBe(12);
    expect(
      response.conversations[0]?.recentInvocations[0]?.tUpstreamTtfbMs,
    ).toBe(13);
    expect(
      response.conversations[0]?.recentInvocations[0]?.tUpstreamStreamMs,
    ).toBe(14);
    expect(response.conversations[0]?.recentInvocations[0]?.tRespParseMs).toBe(
      15,
    );
    expect(response.conversations[0]?.recentInvocations[0]?.tPersistMs).toBe(
      16,
    );
    expect(response.conversations[0]?.recentInvocations[0]?.tTotalMs).toBe(91);
  });

  it("supports the precise 5-minute working-conversation query contract", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      expect(String(input)).toContain(
        "/api/stats/prompt-cache-conversations?activityMinutes=5",
      );
      return new Response(
        JSON.stringify({
          rangeStart: "2026-03-10T23:55:00Z",
          rangeEnd: "2026-03-11T00:00:00Z",
          selectionMode: "activityWindow",
          selectedLimit: null,
          selectedActivityHours: null,
          selectedActivityMinutes: 5,
          implicitFilter: {
            kind: null,
            filteredCount: 0,
          },
          conversations: [],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    });
    vi.stubGlobal("fetch", fetchMock as typeof fetch);

    const response = await fetchPromptCacheConversations({
      mode: "activityWindow",
      activityMinutes: 5,
    });

    expect(response.selectionMode).toBe("activityWindow");
    expect(response.selectedActivityHours).toBeNull();
    expect(response.selectedActivityMinutes).toBe(5);
  });
});
