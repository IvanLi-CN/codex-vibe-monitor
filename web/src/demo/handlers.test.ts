import { setupServer } from "msw/node";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { apiHandlers, demoAttemptPhase } from "./handlers";
import { DEMO_API_KEY_DISPLAY_NAMES, demoModel } from "./model";

const server = setupServer(...apiHandlers);

beforeAll(() => server.listen({ onUnhandledRequest: "error" }));
afterEach(() => {
  demoModel.setScene("operational");
  demoModel.reset();
});
afterAll(() => server.close());

describe("demo MSW handlers", () => {
  it("serves the checked-in release version without exposing demo-only labels", async () => {
    const response = await fetch("http://demo.invalid/api/version");
    expect(response.ok).toBe(true);
    await expect(response.json()).resolves.toEqual({ backend: "0.2.0", frontend: "0.2.0" });
  });

  it("treats zero-millisecond TTFT as responding in account attempts", () => {
    expect(demoAttemptPhase("running", 0)).toBe("responding");
    expect(demoAttemptPhase("running", null)).toBe("requesting");
    expect(demoAttemptPhase("running", -1)).toBe("requesting");
  });

  it("serves account request attempts with an in-flight TTFT and no completed response duration", async () => {
    const response = await fetch(
      "http://demo.invalid/api/pool/upstream-accounts/101/call-attempts?page=1&pageSize=50",
    );
    const payload = (await response.json()) as {
      items: Array<{
        attemptId: string;
        phase: string | null;
        firstTokenMs: number | null;
        streamLatencyMs: number | null;
      }>;
      total: number;
    };

    expect(response.ok).toBe(true);
    expect(payload.total).toBeGreaterThan(0);
    expect(payload.items).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          phase: "responding",
          firstTokenMs: 705,
          streamLatencyMs: null,
        }),
      ]),
    );
  });

  it("attributes retry timing only to the final demo attempt", async () => {
    const response = await fetch(
      "http://demo.invalid/api/pool/upstream-accounts/102/call-attempts?page=1&pageSize=200",
    );
    const payload = (await response.json()) as {
      items: Array<{
        invokeId: string;
        attemptIndex: number;
        firstTokenMs: number | null;
        streamLatencyMs: number | null;
      }>;
    };

    expect(response.ok).toBe(true);
    const finalRetry = payload.items.find((item) => item.attemptIndex > 1);
    const earlierRetry = payload.items.find(
      (item) => item.invokeId === finalRetry?.invokeId && item.attemptIndex === 1,
    );
    expect(earlierRetry).toMatchObject({ firstTokenMs: null, streamLatencyMs: null });
    expect(finalRetry?.firstTokenMs).toEqual(expect.any(Number));
    expect(finalRetry?.streamLatencyMs).toEqual(expect.any(Number));
  });

  it.each([
    ["runtime-pressure-healthy", "healthy"],
    ["runtime-pressure-deferred", "deferred"],
    ["runtime-pressure-degraded", "degraded"],
    ["runtime-pressure-accounting-error", "accounting_error"],
  ] as const)("serves the %s System Status scene", async (scene, expectedState) => {
    demoModel.setScene(scene);
    const response = await fetch("http://demo.invalid/api/system/status");
    const payload = (await response.json()) as {
      runtimePressureHealth: {
        state: string;
        dashboardProjection: { livePathDbReadCount: number };
      };
    };
    expect(payload.runtimePressureHealth.state).toBe(expectedState);
    expect(payload.runtimePressureHealth.dashboardProjection.livePathDbReadCount).toBe(0);
  });

  it("serves deterministic dashboard activity in the shape used by the production normalizer", async () => {
    const response = await fetch("http://demo.invalid/api/stats/dashboard-activity?range=today");
    const payload = (await response.json()) as {
      summary: {
        stats: {
          totalCount: number;
          usageBreakdown: {
            models: Array<{ model: string; reasoningEffort: string | null }>;
          };
        };
        modelPerformance: {
          available: boolean;
          total: {
            tokensPerMinute: number;
            wallClockUsageDurationMs: number;
            cumulativeUsageDurationMs: number;
            parallelism: number;
          };
          models: Array<{
            model: string;
            reasoningEffort: string | null;
            wallClockUsageDurationMs: number;
            cumulativeUsageDurationMs: number;
            parallelism: number;
          }>;
        };
      };
    };

    expect(response.ok).toBe(true);
    expect(payload.summary.stats.totalCount).toBe(12_846);
    expect(payload.summary.stats.usageBreakdown.models).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ model: "gpt-5.6-sol", reasoningEffort: "high" }),
        expect.objectContaining({ model: "gpt-5.6-sol", reasoningEffort: "medium" }),
        expect.objectContaining({ model: "gpt-5.6-terra", reasoningEffort: null }),
      ]),
    );
    expect(payload.summary.modelPerformance).toMatchObject({
      available: true,
      total: {
        tokensPerMinute: expect.any(Number),
        wallClockUsageDurationMs: expect.any(Number),
        cumulativeUsageDurationMs: expect.any(Number),
        parallelism: expect.any(Number),
      },
    });
    expect(payload.summary.modelPerformance.models).toEqual([
      expect.objectContaining({
        model: "gpt-5.6-sol",
        reasoningEffort: "high",
        wallClockUsageDurationMs: expect.any(Number),
        cumulativeUsageDurationMs: expect.any(Number),
        parallelism: expect.any(Number),
      }),
      expect.objectContaining({
        model: "gpt-5.6-sol",
        reasoningEffort: "medium",
        wallClockUsageDurationMs: expect.any(Number),
        cumulativeUsageDurationMs: expect.any(Number),
        parallelism: expect.any(Number),
      }),
      expect.objectContaining({
        model: "gpt-5.6-terra",
        reasoningEffort: null,
        wallClockUsageDurationMs: expect.any(Number),
        cumulativeUsageDurationMs: expect.any(Number),
        parallelism: expect.any(Number),
      }),
    ]);
  });

  it("derives live summary counters from the same invocation ledger as routing attempts", async () => {
    const [summaryResponse, recordsResponse] = await Promise.all([
      fetch("http://demo.invalid/api/stats/summary"),
      fetch("http://demo.invalid/api/invocations?pageSize=200"),
    ]);
    const summary = (await summaryResponse.json()) as {
      totalCount: number;
      successCount: number;
      failureCount: number;
      totalTokens: number;
      totalCost: number;
      token: { cacheInputTokens: number };
    };
    const payload = (await recordsResponse.json()) as {
      records: Array<{
        status: string;
        totalTokens: number | null;
        cost: number | null;
        cacheInputTokens: number | null;
      }>;
    };

    expect(summaryResponse.ok).toBe(true);
    expect(summary.totalCount).toBe(payload.records.length);
    expect(summary.successCount).toBe(
      payload.records.filter((record) => record.status === "success").length,
    );
    expect(summary.failureCount).toBe(
      payload.records.filter((record) => !["success", "running"].includes(record.status)).length,
    );
    expect(summary.totalTokens).toBe(
      payload.records.reduce((total, record) => total + (record.totalTokens ?? 0), 0),
    );
    expect(summary.totalCost).toBeCloseTo(
      payload.records.reduce((total, record) => total + (record.cost ?? 0), 0),
      4,
    );
    expect(summary.token.cacheInputTokens).toBe(
      payload.records.reduce((total, record) => total + (record.cacheInputTokens ?? 0), 0),
    );
  });

  it("serves model-plus-effort breakdowns for dashboard account cards on demand", async () => {
    const response = await fetch(
      "http://demo.invalid/api/stats/dashboard-activity?range=today&includeAccounts=true",
    );
    const payload = (await response.json()) as {
      summary: {
        stats: {
          totalCount: number;
          successCount: number;
          failureCount: number;
          totalTokens: number;
          totalCost: number;
        };
      };
      accounts: Array<{
        displayName: string;
        requestCount: number;
        successCount: number;
        failureCount: number;
        totalTokens: number;
        totalCost: number;
        usageBreakdown: { models: Array<{ model: string; reasoningEffort: string | null }> };
        modelPerformance: { models: Array<{ model: string; reasoningEffort: string | null }> };
      }>;
    };

    expect(response.ok).toBe(true);
    expect(payload.accounts).toHaveLength(12);
    expect(payload.accounts[0]).toMatchObject({ displayName: "alpha@demo.invalid" });
    expect(payload.accounts[0]?.usageBreakdown.models).toEqual([
      expect.objectContaining({ model: "gpt-5.6-sol", reasoningEffort: "high" }),
      expect.objectContaining({ model: "gpt-5.6-sol", reasoningEffort: "medium" }),
    ]);
    expect(payload.accounts[0]?.modelPerformance.models).toEqual([
      expect.objectContaining({ model: "gpt-5.6-sol", reasoningEffort: "high" }),
      expect.objectContaining({ model: "gpt-5.6-sol", reasoningEffort: "medium" }),
    ]);
    expect(payload.summary.stats.totalCount).toBe(
      payload.accounts.reduce((total, account) => total + account.requestCount, 0),
    );
    expect(payload.summary.stats.successCount).toBe(
      payload.accounts.reduce((total, account) => total + account.successCount, 0),
    );
    expect(payload.summary.stats.failureCount).toBe(
      payload.accounts.reduce((total, account) => total + account.failureCount, 0),
    );
    expect(payload.summary.stats.totalTokens).toBe(
      payload.accounts.reduce((total, account) => total + account.totalTokens, 0),
    );
    expect(payload.summary.stats.totalCost).toBeCloseTo(
      payload.accounts.reduce((total, account) => total + account.totalCost, 0),
      2,
    );
  });

  it("serves dashboard account summaries and snapshot-bound recent rows in separate phases", async () => {
    const summaryResponse = await fetch(
      "http://demo.invalid/api/stats/dashboard-activity?range=today&includeAccounts=true&includeRecent=false",
    );
    const summary = (await summaryResponse.json()) as {
      snapshotId: number;
      rangeStart: string;
      rangeEnd: string;
      accounts: Array<{ recentInvocations: unknown[] }>;
    };
    const recentResponse = await fetch(
      `http://demo.invalid/api/stats/dashboard-activity/recent?rangeStart=${encodeURIComponent(summary.rangeStart)}&rangeEnd=${encodeURIComponent(summary.rangeEnd)}&snapshotId=${summary.snapshotId}&recentLimit=4`,
    );
    const recent = (await recentResponse.json()) as {
      snapshotId: number;
      accounts: Array<{ recentInvocations: unknown[] }>;
    };

    expect(summary.accounts.every((account) => account.recentInvocations.length === 0)).toBe(true);
    expect(recent.snapshotId).toBe(summary.snapshotId);
    expect(recent.accounts[0]?.recentInvocations.length).toBeGreaterThan(0);
  });

  it("accepts Pages-scoped API paths so requests remain inside the demo worker scope", async () => {
    const response = await fetch(
      "http://demo.invalid/codex-vibe-monitor/demo/api/stats/dashboard-activity?range=today",
    );
    const payload = (await response.json()) as { summary: { stats: { totalCount: number } } };

    expect(response.ok).toBe(true);
    expect(payload.summary.stats.totalCount).toBe(12_846);
  });

  it("does not retain sensitive settings input", async () => {
    const response = await fetch("http://demo.invalid/api/settings/proxy", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        enabledModels: ["gpt-5.6-sol"],
        apiKey: "input-must-not-return",
        refreshToken: "token-must-not-return",
      }),
    });
    const body = await response.text();

    expect(response.ok).toBe(true);
    expect(body).not.toContain("input-must-not-return");
    expect(body).not.toContain("token-must-not-return");
  });

  it("creates a deterministic external key result without retaining the submitted name", async () => {
    const response = await fetch("http://demo.invalid/api/settings/external-api-keys", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: "submitted-name-must-not-persist" }),
    });
    const payload = (await response.json()) as { key: { name: string }; secret: string };
    const listing = await fetch("http://demo.invalid/api/settings/external-api-keys");
    const listingBody = await listing.text();

    expect(response.status).toBe(201);
    expect(payload.key.name).toBe("Synthetic integration 7");
    expect(payload.secret).toBe("cvm-synthetic-key-not-valid");
    expect(listingBody).toContain("Synthetic integration 7");
    expect(listingBody).not.toContain("submitted-name-must-not-persist");
  });

  it("serves a dense, linked operational dataset across records, pool, detail, and system pages", async () => {
    const [recordsResponse, accountsResponse, eventsResponse, tasksResponse] = await Promise.all([
      fetch("http://demo.invalid/api/invocations?pageSize=50"),
      fetch("http://demo.invalid/api/pool/upstream-accounts?includeAll=true&pageSize=50"),
      fetch("http://demo.invalid/api/pool/upstream-account-events?pageSize=20"),
      fetch("http://demo.invalid/api/system/tasks?pageSize=20"),
    ]);
    const records = (await recordsResponse.json()) as {
      records: Array<{
        id: number;
        invokeId: string;
        upstreamAccountId: number;
        promptCacheKey: string;
      }>;
    };
    const accounts = (await accountsResponse.json()) as {
      items: Array<{ id: number; groupName: string | null; boundProxyKeys: string[] }>;
    };
    const events = (await eventsResponse.json()) as {
      items: Array<{
        action: string;
        result: string;
        failureKind: string | null;
        model: string | null;
        accountDisplayName: string;
        forwardProxyKey: string | null;
      }>;
    };
    const tasks = (await tasksResponse.json()) as { items: Array<{ status: string }> };
    const selectedRecord = records.records.find((record) => record.upstreamAccountId === 102);

    expect(records.records).toHaveLength(50);
    expect(accounts.items).toHaveLength(15);
    expect(accounts.items.some((account) => account.groupName === "production")).toBe(true);
    expect(accounts.items.some((account) => account.groupName === "recovery")).toBe(true);
    expect(events.items).toHaveLength(15);
    expect(
      events.items.some(
        (event) =>
          event.action === "model_route_cooldown" &&
          event.result === "failed" &&
          event.failureKind === "model" &&
          event.model === "gpt-5.4-mini",
      ),
    ).toBe(true);
    expect(tasks.items.map((item) => item.status)).toEqual(
      expect.arrayContaining(["success", "running", "failed"]),
    );
    expect(selectedRecord).toBeDefined();

    const locatedResponse = await fetch(
      `http://demo.invalid/api/invocations?invokeId=${encodeURIComponent(selectedRecord?.invokeId ?? "")}&pageSize=1`,
    );
    const located = (await locatedResponse.json()) as {
      records: Array<{ invokeId: string }>;
    };
    expect(located.records).toHaveLength(1);
    expect(located.records[0]?.invokeId).toBe(selectedRecord?.invokeId);

    const [detailResponse, attemptsResponse, accountResponse] = await Promise.all([
      fetch(`http://demo.invalid/api/invocations/${selectedRecord?.id}/detail`),
      fetch(`http://demo.invalid/api/invocations/${selectedRecord?.invokeId}/pool-attempts`),
      fetch(
        `http://demo.invalid/api/pool/upstream-accounts/${selectedRecord?.upstreamAccountId}?includeRecentActions=true`,
      ),
    ]);
    const detail = (await detailResponse.json()) as {
      id: number;
      abnormalResponseBody: { available: boolean };
    };
    const attempts = (await attemptsResponse.json()) as Array<{
      invokeId: string;
      upstreamAccountId: number;
    }>;
    const account = (await accountResponse.json()) as {
      id: number;
      history: unknown[];
      recentActions: unknown[];
    };

    expect(detail.id).toBe(selectedRecord?.id);
    expect(attempts[0]).toMatchObject({
      invokeId: selectedRecord?.invokeId,
      upstreamAccountId: selectedRecord?.upstreamAccountId,
    });
    expect(account.id).toBe(selectedRecord?.upstreamAccountId);
    expect(account.history).toHaveLength(8);
    expect(account.recentActions.length).toBeGreaterThan(0);
  });

  it("scopes invocation summaries to the same conversation filters as invocation lists", async () => {
    const [summaryResponse, listResponse] = await Promise.all([
      fetch("http://demo.invalid/api/invocations/summary?promptCacheKey=demo-conversation-a"),
      fetch("http://demo.invalid/api/invocations?promptCacheKey=demo-conversation-a&pageSize=50"),
    ]);
    const summary = (await summaryResponse.json()) as {
      totalCount: number;
      token: { totalTokens: number };
    };
    const list = (await listResponse.json()) as {
      total: number;
      records: Array<{ totalTokens?: number }>;
    };

    expect(summaryResponse.ok).toBe(true);
    expect(listResponse.ok).toBe(true);
    expect(summary.totalCount).toBe(list.total);
    expect(summary.token.totalTokens).toBe(
      list.records.reduce((total, record) => total + (record.totalTokens ?? 0), 0),
    );
  });

  it("returns populated proxy and prompt-cache surfaces rather than empty placeholders", async () => {
    const [proxyResponse, cacheResponse, proxyHistoryResponse] = await Promise.all([
      fetch("http://demo.invalid/api/stats/forward-proxy"),
      fetch("http://demo.invalid/api/stats/prompt-cache-conversations?limit=50"),
      fetch("http://demo.invalid/api/stats/forward-proxy/timeseries?range=today&bucket=1h"),
    ]);
    const proxy = (await proxyResponse.json()) as {
      nodes: Array<{ last24h: unknown[]; weight24h: unknown[] }>;
    };
    const cache = (await cacheResponse.json()) as {
      conversations: Array<{ upstreamAccounts: unknown[]; recentInvocations: unknown[] }>;
    };
    const proxyHistory = (await proxyHistoryResponse.json()) as {
      nodes: Array<{ buckets: unknown[] }>;
    };

    expect(proxy.nodes).toHaveLength(5);
    expect(proxy.nodes.every((node) => node.last24h.length > 0 && node.weight24h.length > 0)).toBe(
      true,
    );
    expect(cache.conversations).toHaveLength(11);
    expect(cache.conversations[0]?.upstreamAccounts.length).toBeGreaterThan(0);
    expect(cache.conversations[0]?.recentInvocations.length).toBeGreaterThan(0);
    expect(proxyHistory.nodes.every((node) => node.buckets.length > 0)).toBe(true);
  });

  it("uses supported localized source values for account health events", async () => {
    const response = await fetch(
      "http://demo.invalid/api/pool/upstream-accounts/102?includeRecentActions=true",
    );
    const account = (await response.json()) as {
      recentActions: Array<{ action: string; source: string }>;
    };

    expect(response.ok).toBe(true);
    expect(account.recentActions).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          action: "model_route_cooldown",
          source: "call",
          httpStatus: 502,
          model: "gpt-5.4-mini",
        }),
      ]),
    );
    expect(account.recentActions.map((event) => event.source)).not.toEqual(
      expect.arrayContaining(["operator", "maintenance_scheduler"]),
    );
  });

  it("serves API Key model routing snapshots and exact-model history", async () => {
    const [liveResponse, historyResponse, nextHistoryResponse] = await Promise.all([
      fetch("http://demo.invalid/api/pool/model-routing-live?window=1h&limit=100"),
      fetch(
        "http://demo.invalid/api/pool/upstream-accounts/102/model-routing-events?model=gpt-5.4-mini&pageSize=2",
      ),
      fetch(
        "http://demo.invalid/api/pool/upstream-accounts/102/model-routing-events?model=gpt-5.4-mini&cursor=demo-model-routing-page-2&pageSize=2",
      ),
    ]);
    const live = (await liveResponse.json()) as {
      groups: Array<{
        accounts: Array<{
          accountId: number;
          accountDisplayName?: string;
          accountGroupName?: unknown;
        }>;
      }>;
      records: Array<{
        kind: string;
        attemptIndex?: number;
        invokeId?: string;
        accountDisplayName?: string;
        accountGroupName?: unknown;
      }>;
    };
    const history = (await historyResponse.json()) as {
      items: Array<{ model: string; kind: string; attemptIndex?: number }>;
      nextCursor?: string | null;
    };
    const nextHistory = (await nextHistoryResponse.json()) as {
      items: Array<{ model: string; kind: string }>;
      nextCursor?: string | null;
    };

    expect(liveResponse.ok).toBe(true);
    expect(historyResponse.ok).toBe(true);
    expect(nextHistoryResponse.ok).toBe(true);
    const account102 = demoModel.snapshot.accounts.find((account) => account.id === 102) as
      | { displayName: string }
      | undefined;
    expect(account102).toBeDefined();
    expect(account102?.displayName).toBe(DEMO_API_KEY_DISPLAY_NAMES[102]);
    expect(
      live.groups.flatMap((group) => group.accounts.map((account) => account.accountId)),
    ).toEqual(expect.arrayContaining([102, 106]));
    expect(live.groups.flatMap((group) => group.accounts)).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          accountId: 102,
          accountDisplayName: account102?.displayName,
        }),
      ]),
    );
    const routeAccountNames = live.groups
      .flatMap((group) => group.accounts)
      .map((account) => account.accountDisplayName)
      .filter((name): name is string => typeof name === "string");
    expect(routeAccountNames).toEqual(
      expect.arrayContaining(Object.values(DEMO_API_KEY_DISPLAY_NAMES)),
    );
    expect(routeAccountNames).not.toEqual(
      expect.arrayContaining([expect.stringMatching(/^API Key #/)]),
    );
    expect(live.records).toEqual(
      expect.arrayContaining([expect.objectContaining({ kind: "attempt", attemptIndex: 2 })]),
    );
    expect(live.records).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          accountDisplayName: account102?.displayName,
        }),
      ]),
    );
    expect(live.groups.flatMap((group) => group.accounts)).not.toContainEqual(
      expect.objectContaining({ accountGroupName: expect.anything() }),
    );
    expect(live.records).not.toContainEqual(
      expect.objectContaining({ accountGroupName: expect.anything() }),
    );
    expect(live.records.filter((record) => record.kind === "event")).not.toContainEqual(
      expect.objectContaining({ invokeId: expect.any(String) }),
    );
    expect(history.items).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ model: "gpt-5.4-mini", kind: "attempt", attemptIndex: 2 }),
      ]),
    );
    expect(history.nextCursor).toBe("demo-model-routing-page-2");
    expect(nextHistory.items).toEqual(
      expect.arrayContaining([expect.objectContaining({ model: "gpt-5.4-mini", kind: "attempt" })]),
    );
    expect(nextHistory.nextCursor).toBeNull();
  });

  it("filters the live routing fixture by model, current state, time window, and limit", async () => {
    const [filteredResponse, shortWindowResponse, longWindowResponse] = await Promise.all([
      fetch(
        "http://demo.invalid/api/pool/model-routing-live?window=1h&model=gpt-5.4-mini&state=available&limit=1",
      ),
      fetch("http://demo.invalid/api/pool/model-routing-live?window=15m&limit=100"),
      fetch("http://demo.invalid/api/pool/model-routing-live?window=24h&limit=100"),
    ]);
    const filtered = (await filteredResponse.json()) as {
      groups: Array<{
        model: string;
        accounts: Array<{ accountId: number; state: string }>;
      }>;
      records: Array<{ model: string }>;
    };
    const shortWindow = (await shortWindowResponse.json()) as { records: Array<{ id: string }> };
    const longWindow = (await longWindowResponse.json()) as { records: Array<{ id: string }> };

    expect(filteredResponse.ok).toBe(true);
    expect(filtered.groups).toHaveLength(1);
    expect(filtered.groups[0]).toMatchObject({ model: "gpt-5.4-mini" });
    expect(filtered.groups[0]?.accounts).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          accountId: 108,
          state: "available",
        }),
        expect.objectContaining({
          accountId: 115,
          state: "available",
        }),
      ]),
    );
    expect(filtered.records).toHaveLength(1);
    expect(filtered.records[0]).toMatchObject({ model: "gpt-5.4-mini" });
    expect(shortWindow.records.length).toBeLessThan(longWindow.records.length);
  });

  it("derives route attempts from the same invocation records shown in the demo", async () => {
    const [routingResponse, invocationsResponse] = await Promise.all([
      fetch("http://demo.invalid/api/pool/model-routing-live?window=24h&limit=100"),
      fetch("http://demo.invalid/api/invocations?pageSize=200"),
    ]);
    const routing = (await routingResponse.json()) as {
      records: Array<{
        kind: string;
        invokeId?: string;
        upstreamAccountId?: number;
        accountId?: number;
        model: string;
        status?: string;
        attemptIndex?: number;
      }>;
    };
    const invocationList = (await invocationsResponse.json()) as {
      records: Array<{
        invokeId: string;
        upstreamAccountId: number;
        model: string;
        status: string;
      }>;
    };
    const invocationsById = new Map(
      invocationList.records.map((invocation) => [invocation.invokeId, invocation]),
    );
    const attempts = routing.records.filter((record) => record.kind === "attempt");

    expect(routing.records).toHaveLength(100);
    expect(attempts.length).toBeGreaterThan(0);
    for (const attempt of attempts) {
      const invocation = invocationsById.get(attempt.invokeId ?? "");
      expect(invocation).toBeDefined();
      expect(invocation).toMatchObject({
        upstreamAccountId: attempt.accountId,
        model: attempt.model,
      });
    }

    const terminalAttempts = new Map<string, (typeof attempts)[number]>();
    for (const attempt of attempts) {
      const key = attempt.invokeId ?? "";
      const previous = terminalAttempts.get(key);
      if ((attempt.attemptIndex ?? 0) > (previous?.attemptIndex ?? 0)) {
        terminalAttempts.set(key, attempt);
      }
    }
    for (const attempt of terminalAttempts.values()) {
      expect(invocationsById.get(attempt.invokeId ?? "")).toMatchObject({ status: attempt.status });
    }
  });

  it("serves shareable dashboard invocation detail data for unavailable request-body playback", async () => {
    const [detailResponse, requestBodyResponse, responseBodyResponse, attemptResponseBodyResponse] =
      await Promise.all([
        fetch("http://demo.invalid/api/invocations/9002/workflow-detail"),
        fetch("http://demo.invalid/api/invocations/9002/request-body"),
        fetch("http://demo.invalid/api/invocations/9002/response-body"),
        fetch("http://demo.invalid/api/invocations/9002/attempts/qPvNNAK8/response-body"),
      ]);
    const detail = (await detailResponse.json()) as {
      hero: { invokeId: string; finalStatus: string };
      timeline: Array<{
        kind: string;
        attempt?: {
          attemptId?: string | null;
          requestSummary?: {
            bodyCapture?: { availableAtInvocationLevel?: boolean | null };
            compression?: { algorithm?: string | null; ratioPct?: number | null };
          } | null;
        } | null;
      }>;
    };
    const requestBody = (await requestBodyResponse.json()) as {
      available: boolean;
      unavailableReason?: string | null;
    };
    const responseBody = (await responseBodyResponse.json()) as {
      available: boolean;
      bodySize?: number | null;
    };
    const attemptResponseBody = (await attemptResponseBodyResponse.json()) as {
      available: boolean;
      captureSource?: string | null;
      headers?: { upstreamRequestId?: string | null };
    };

    expect(detailResponse.ok).toBe(true);
    expect(requestBodyResponse.ok).toBe(true);
    expect(responseBodyResponse.ok).toBe(true);
    expect(attemptResponseBodyResponse.ok).toBe(true);
    expect(detail.hero).toMatchObject({
      invokeId: "demo-invocation-9002",
      finalStatus: "completed",
    });
    const attemptEntry = detail.timeline.find((entry) => entry.kind === "attempt");
    expect(attemptEntry?.attempt?.attemptId).toBe("qPvNNAK8");
    expect(attemptEntry?.attempt?.requestSummary?.bodyCapture?.availableAtInvocationLevel).toBe(
      false,
    );
    expect(attemptEntry?.attempt?.requestSummary?.compression).toMatchObject({
      algorithm: "zstd",
      ratioPct: -63,
    });
    expect(requestBody).toMatchObject({
      available: false,
      unavailableReason: "missing_body",
    });
    expect(responseBody).toMatchObject({
      available: true,
      bodySize: 138_649,
    });
    expect(attemptResponseBody).toMatchObject({
      available: true,
      captureSource: "attempt_raw_file",
      headers: { upstreamRequestId: "up_demo_9002_1" },
    });

    const missingAttemptResponse = await fetch(
      "http://demo.invalid/api/invocations/9002/attempts/missing-attempt/response-body",
    );
    expect(missingAttemptResponse.status).toBe(404);
  });

  it("keeps Demo workflow response duration unavailable while an invocation is responding", async () => {
    const response = await fetch("http://demo.invalid/api/invocations/9001/workflow-detail");
    const detail = (await response.json()) as {
      timeline: Array<{
        kind: string;
        attempt?: {
          phase?: string | null;
          finishedAt?: string | null;
          firstTokenMs?: number | null;
          streamLatencyMs?: number | null;
        } | null;
      }>;
    };

    const attempt = detail.timeline.find((entry) => entry.kind === "attempt")?.attempt;
    expect(response.ok).toBe(true);
    expect(attempt).toMatchObject({
      phase: "responding",
      finishedAt: null,
      firstTokenMs: 705,
      streamLatencyMs: null,
    });
  });

  it("fails closed instead of returning a real network response in network-failure scene", async () => {
    demoModel.setScene("network-failure");

    await expect(fetch("http://demo.invalid/api/stats/summary")).rejects.toThrow();
  });
});
