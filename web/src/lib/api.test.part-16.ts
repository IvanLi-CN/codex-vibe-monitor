import { expect, it, vi } from "vitest";
import {
  DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS,
  fetchInvocationRecordLocation,
  fetchInvocationRecords,
  fetchUpstreamAccountDetail,
  fetchUpstreamAccounts,
  fetchUpstreamAccountWindowUsage,
} from "./api";

it("adds invokeId to invocation records query parameters", async () => {
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    expect(url).toContain("/api/invocations?");
    expect(url).toContain("invokeId=invoke-123");
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

  await fetchInvocationRecords({ invokeId: "invoke-123" });

  expect(fetchMock).toHaveBeenCalledTimes(1);
});
it("encodes the account-scoped invocation locator query", async () => {
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    expect(url).toContain("/api/invocations/locate?");
    expect(url).toContain("invokeId=invoke-anchor");
    expect(url).toContain("upstreamAccountId=42");
    expect(url).toContain("pageSize=50");
    return new Response(
      JSON.stringify({
        anchorId: "anchor-test-001",
        snapshotId: 7,
        total: 1,
        page: 1,
        pageSize: 50,
        records: [],
        targetIndex: 0,
        targetAbsoluteIndex: 0,
      }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    );
  });
  vi.stubGlobal("fetch", fetchMock as typeof fetch);

  await fetchInvocationRecordLocation({
    invokeId: "invoke-anchor",
    upstreamAccountId: 42,
  });

  expect(fetchMock).toHaveBeenCalledTimes(1);
});
it("adds anchor, snapshot, sticky key, and account to invocation records queries", async () => {
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    expect(url).toContain("/api/invocations?");
    expect(url).toContain("stickyKey=sticky-001");
    expect(url).toContain("upstreamAccountId=42");
    expect(url).toContain("snapshotId=7");
    expect(url).toContain("anchorId=anchor-test-001");
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
    snapshotId: 7,
    anchorId: "anchor-test-001",
  });

  expect(fetchMock).toHaveBeenCalledTimes(1);
});
it("adds extended diagnostics filters to invocation records queries", async () => {
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    expect(url).toContain("/api/invocations?");
    expect(url).toContain("upstreamScope=internal");
    expect(url).toContain("proxyDisplayName=tokyo-edge");
    expect(url).toContain("transport=websocket");
    expect(url).toContain("serviceTier=priority");
    expect(url).toContain("reasoningEffort=high");
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
    upstreamScope: "internal",
    proxyDisplayName: "tokyo-edge",
    transport: "websocket",
    serviceTier: "priority",
    reasoningEffort: "high",
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
      primarySyncIntervalSecs: DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS.primarySyncIntervalSecs,
      secondarySyncIntervalSecs:
        DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS.secondarySyncIntervalSecs,
      priorityAvailableAccountCap:
        DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS.priorityAvailableAccountCap,
    },
    timeouts: {
      responsesFirstByteTimeoutSecs: 120,
      compactFirstByteTimeoutSecs: 300,
      imageFirstByteTimeoutSecs: 300,
      responsesStreamTimeoutSecs: 300,
      compactStreamTimeoutSecs: 300,
    },
    requestCompressionAlgorithm: "identity",
    requestCompressionLevelPreset: "balanced",
    codexImagegenRewriteMode: "keep_original",
    cacheHitProtection: {
      enabled: false,
      lowHitRateThresholdPercent: 10,
      overflowMode: "queue",
      minimumInputTokens: 3840,
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
              imageFirstByteTimeoutSecs: 480,
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
    imageFirstByteTimeoutSecs: 480,
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
                reason: "No available channel for compact model gpt-5.4-openai-compact",
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
  let requestedUrl = "";
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      requestedUrl = String(input);
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
  expect(requestedUrl).toContain("/api/pool/upstream-accounts/9");
  expect(requestedUrl).not.toContain("includeRecentActions");
});
it("adds includeRecentActions when requested for upstream account detail", async () => {
  let requestedUrl = "";
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      requestedUrl = String(input);
      return new Response(
        JSON.stringify({
          id: 9,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Detail OAuth",
          isMother: false,
          status: "active",
          enabled: true,
          history: [],
          recentActions: [],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    }) as typeof fetch,
  );

  await fetchUpstreamAccountDetail(9, { includeRecentActions: true });

  expect(requestedUrl).toContain("/api/pool/upstream-accounts/9?includeRecentActions=true");
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
                    allowCutOut: true,
                    allowCutIn: true,
                    priorityTier: "primary",
                    fastModeRewriteMode: "force_add",
                    upstream429RetryEnabled: true,
                    upstream429MaxRetries: 3,
                  },
                },
              ],
              effectiveRoutingRule: {
                allowCutOut: true,
                allowCutIn: true,
                priorityTier: "fallback",
                fastModeRewriteMode: "force_remove",
                upstream429RetryEnabled: true,
                upstream429MaxRetries: 5,
                sourceTagIds: [31],
                sourceTagNames: ["priority-route"],
              },
            },
          ],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    }) as typeof fetch,
  );

  const response = await fetchUpstreamAccounts();

  expect(response.items[0]?.tags?.[0]?.routingRule.fastModeRewriteMode).toBe("force_add");
  expect(response.items[0]?.effectiveRoutingRule?.fastModeRewriteMode).toBe("force_remove");
  expect(response.items[0]?.effectiveRoutingRule?.upstream429RetryEnabled).toBe(true);
  expect(response.items[0]?.effectiveRoutingRule?.upstream429MaxRetries).toBe(5);
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
                allowCutOut: true,
                allowCutIn: true,
                priorityTier: "normal",
              },
            },
          ],
          effectiveRoutingRule: {
            allowCutOut: true,
            allowCutIn: true,
            priorityTier: "normal",
            fastModeRewriteMode: "unexpected",
            sourceTagIds: [41],
            sourceTagNames: ["legacy-tag"],
          },
          history: [],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    }) as typeof fetch,
  );

  const response = await fetchUpstreamAccountDetail(9);

  expect(response.tags[0]?.routingRule.fastModeRewriteMode).toBe("keep_original");
  expect(response.effectiveRoutingRule?.fastModeRewriteMode).toBe("keep_original");
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
it("serializes window-usage batch requests and normalizes the response", async () => {
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    expect(String(input)).toContain("/api/pool/upstream-accounts/window-usage");
    expect(init?.method).toBe("POST");
    expect(init?.body).toBe(JSON.stringify({ accountIds: [3, 7] }));
    return new Response(
      JSON.stringify({
        items: [
          {
            accountId: 3,
            primaryActualUsage: {
              requestCount: 11,
              totalTokens: 64120,
              totalCost: 0.6123,
              inputTokens: 38200,
              outputTokens: 21120,
              cacheInputTokens: 4800,
            },
            secondaryActualUsage: null,
          },
          {
            accountId: 7,
            primaryActualUsage: null,
            secondaryActualUsage: {
              requestCount: 52,
              totalTokens: 201440,
              totalCost: 1.8821,
              inputTokens: 110200,
              outputTokens: 78240,
              cacheInputTokens: 13000,
            },
          },
        ],
      }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    );
  });
  vi.stubGlobal("fetch", fetchMock as typeof fetch);

  const response = await fetchUpstreamAccountWindowUsage([3, 7]);

  expect(response.items).toEqual([
    {
      accountId: 3,
      primaryActualUsage: {
        requestCount: 11,
        totalTokens: 64120,
        totalCost: 0.6123,
        inputTokens: 38200,
        outputTokens: 21120,
        cacheInputTokens: 4800,
      },
      secondaryActualUsage: null,
    },
    {
      accountId: 7,
      primaryActualUsage: null,
      secondaryActualUsage: {
        requestCount: 52,
        totalTokens: 201440,
        totalCost: 1.8821,
        inputTokens: 110200,
        outputTokens: 78240,
        cacheInputTokens: 13000,
      },
    },
  ]);
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
