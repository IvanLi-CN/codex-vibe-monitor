import { afterEach, describe, expect, it, vi } from "vitest";
import {
  fetchInvocationRecords,
  fetchForwardProxyLiveStats,
  fetchSettings,
  fetchSummary,
  fetchUpstreamAccounts,
  fetchUpstreamStickyConversations,
  updateProxySettings,
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
});

describe("proxy settings normalization", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("defaults invalid fast rewrite mode to disabled when fetching settings", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            proxy: {
              hijackEnabled: true,
              mergeUpstreamEnabled: true,
              defaultHijackEnabled: false,
              models: ["gpt-5.3-codex"],
              enabledModels: ["gpt-5.3-codex"],
              fastModeRewriteMode: "wat",
              upstream429MaxRetries: 99,
            },
            forwardProxy: {
              proxyUrls: [],
              subscriptionUrls: [],
              subscriptionUpdateIntervalSecs: 3600,
              insertDirect: true,
              nodes: [],
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
    expect(settings.proxy.fastModeRewriteMode).toBe("disabled");
    expect(settings.proxy.upstream429MaxRetries).toBe(5);
  });

  it("defaults invalid upstream 429 retry count to 3 when fetching settings", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            proxy: {
              hijackEnabled: true,
              mergeUpstreamEnabled: true,
              defaultHijackEnabled: false,
              models: ["gpt-5.3-codex"],
              enabledModels: ["gpt-5.3-codex"],
              fastModeRewriteMode: "disabled",
              upstream429MaxRetries: "bad",
            },
            forwardProxy: {
              proxyUrls: [],
              subscriptionUrls: [],
              subscriptionUpdateIntervalSecs: 3600,
              insertDirect: true,
              nodes: [],
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
    expect(settings.proxy.upstream429MaxRetries).toBe(3);
  });

  it("sends upstream 429 retry count in proxy settings update payload", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, init?: RequestInit) => {
        expect(init?.method).toBe("PUT");
        expect(typeof init?.body).toBe("string");
        expect(JSON.parse(String(init?.body))).toEqual({
          hijackEnabled: true,
          mergeUpstreamEnabled: false,
          enabledModels: ["gpt-5.3-codex"],
          fastModeRewriteMode: "force_priority",
          upstream429MaxRetries: 4,
        });
        return new Response(
          JSON.stringify({
            hijackEnabled: true,
            mergeUpstreamEnabled: false,
            defaultHijackEnabled: false,
            models: ["gpt-5.3-codex"],
            enabledModels: ["gpt-5.3-codex"],
            fastModeRewriteMode: "force_priority",
            upstream429MaxRetries: 4,
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      },
    );
    vi.stubGlobal("fetch", fetchMock as typeof fetch);

    const response = await updateProxySettings({
      hijackEnabled: true,
      mergeUpstreamEnabled: false,
      enabledModels: ["gpt-5.3-codex"],
      fastModeRewriteMode: "force_priority",
      upstream429MaxRetries: 4,
    });

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(response.fastModeRewriteMode).toBe("force_priority");
    expect(response.upstream429MaxRetries).toBe(4);
  });
});

describe("account pool frontend API helpers", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("adds upstreamScope to invocation records query parameters", async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      expect(url).toContain("/api/invocations?");
      expect(url).toContain("upstreamScope=internal");
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

    await fetchInvocationRecords({ upstreamScope: "internal" });

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
      apiKeyConfigured: true,
      maskedApiKey: "pool-live••••••c0de",
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

  it("saves pool routing settings through the dedicated endpoint", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, init?: RequestInit) => {
        expect(String(_input)).toContain("/api/pool/routing-settings");
        expect(init?.method).toBe("PUT");
        expect(JSON.parse(String(init?.body))).toEqual({
          apiKey: "pool-secret",
        });
        return new Response(
          JSON.stringify({
            apiKeyConfigured: true,
            maskedApiKey: "pool-live••••••cret",
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      },
    );
    vi.stubGlobal("fetch", fetchMock as typeof fetch);

    const response = await updatePoolRoutingSettings({ apiKey: "pool-secret" });

    expect(response.apiKeyConfigured).toBe(true);
    expect(response.maskedApiKey).toBe("pool-live••••••cret");
  });

  it("normalizes sticky key conversations for one upstream account", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            rangeStart: "2026-03-10T00:00:00Z",
            rangeEnd: "2026-03-11T00:00:00Z",
            conversations: [
              {
                stickyKey: "sticky-001",
                requestCount: 2,
                totalTokens: 30,
                totalCost: 0.12,
                createdAt: "2026-03-10T01:00:00Z",
                lastActivityAt: "2026-03-10T02:00:00Z",
                last24hRequests: [
                  {
                    occurredAt: "2026-03-10T02:00:00Z",
                    status: "success",
                    isSuccess: true,
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

    const response = await fetchUpstreamStickyConversations(101, 20);

    expect(response.conversations).toHaveLength(1);
    expect(response.conversations[0]?.stickyKey).toBe("sticky-001");
    expect(
      response.conversations[0]?.last24hRequests[0]?.cumulativeTokens,
    ).toBe(30);
  });
});
