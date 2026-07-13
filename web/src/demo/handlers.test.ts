import { setupServer } from "msw/node";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { apiHandlers } from "./handlers";
import { demoModel } from "./model";

const server = setupServer(...apiHandlers);

beforeAll(() => server.listen({ onUnhandledRequest: "error" }));
afterEach(() => {
  demoModel.setScene("operational");
  demoModel.reset();
});
afterAll(() => server.close());

describe("demo MSW handlers", () => {
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
      };
    };

    expect(response.ok).toBe(true);
    expect(payload.summary.stats.totalCount).toBe(12_846);
    expect(payload.summary.stats.usageBreakdown.models).toEqual([
      expect.objectContaining({ model: "gpt-5.6-sol", reasoningEffort: "high" }),
      expect.objectContaining({ model: "gpt-5.6-sol", reasoningEffort: "medium" }),
      expect.objectContaining({ model: "gpt-5.6-terra", reasoningEffort: null }),
    ]);
  });

  it("serves model-plus-effort breakdowns for dashboard account cards on demand", async () => {
    const response = await fetch(
      "http://demo.invalid/api/stats/dashboard-activity?range=today&includeAccounts=true",
    );
    const payload = (await response.json()) as {
      accounts: Array<{
        displayName: string;
        usageBreakdown: { models: Array<{ model: string; reasoningEffort: string | null }> };
      }>;
    };

    expect(response.ok).toBe(true);
    expect(payload.accounts).toHaveLength(12);
    expect(payload.accounts[0]).toMatchObject({ displayName: "alpha@demo.invalid" });
    expect(payload.accounts[0]?.usageBreakdown.models).toEqual([
      expect.objectContaining({ model: "gpt-5.6-sol", reasoningEffort: "high" }),
      expect.objectContaining({ model: "gpt-5.6-sol", reasoningEffort: "medium" }),
    ]);
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
    expect(payload.key.name).toBe("Demo integration 7");
    expect(payload.secret).toBe("demo-generated-key-not-valid");
    expect(listingBody).toContain("Demo integration 7");
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
      items: Array<{ accountDisplayName: string; forwardProxyKey: string | null }>;
    };
    const tasks = (await tasksResponse.json()) as { items: Array<{ status: string }> };
    const selectedRecord = records.records.find((record) => record.upstreamAccountId === 101);

    expect(records.records).toHaveLength(30);
    expect(accounts.items).toHaveLength(15);
    expect(accounts.items.some((account) => account.groupName === "production")).toBe(true);
    expect(accounts.items.some((account) => account.groupName === null)).toBe(true);
    expect(events.items).toHaveLength(15);
    expect(tasks.items.map((item) => item.status)).toEqual(
      expect.arrayContaining(["success", "running", "failed"]),
    );
    expect(selectedRecord).toBeDefined();

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
      upstreamAccountId: 101,
    });
    expect(account.id).toBe(101);
    expect(account.history).toHaveLength(8);
    expect(account.recentActions.length).toBeGreaterThan(0);
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

  it("fails closed instead of returning a real network response in network-failure scene", async () => {
    demoModel.setScene("network-failure");

    await expect(fetch("http://demo.invalid/api/stats/summary")).rejects.toThrow();
  });
});
