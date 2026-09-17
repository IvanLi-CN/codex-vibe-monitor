import { expect, it, vi } from "vitest";
import { fetchUpstreamAccountActivity } from "./api";

it("normalizes first-response-byte-total activity metrics", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      return new Response(
        JSON.stringify({
          range: "today",
          rangeStart: "2026-06-29T00:00:00Z",
          rangeEnd: "2026-06-29T23:59:59Z",
          accounts: [
            {
              upstreamAccountId: 2890,
              displayName: "dzw",
              requestCount: 13,
              successCount: 10,
              failureCount: 1,
              nonSuccessCount: 1,
              totalTokens: 1498523,
              successTokens: 1498523,
              nonSuccessTokens: 0,
              failureTokens: 0,
              failureCost: 0,
              totalCost: 0.516756,
              cacheHitRate: 0.980421388260307,
              tokensPerMinute: 499507.6666666667,
              spendRate: 0.172252,
              firstByteAvgMs: 0.0050063,
              firstTokenAvgMs: 2867.540251,
              avgTotalMs: 30581.6567545,
              inProgressInvocationCount: 2,
              retryInvocationCount: 0,
              effectiveRoutingRule: {
                priorityTier: "no_new",
                allowCutIn: false,
                fastModeRewriteMode: "force_add",
                concurrencyLimit: 3,
                upstream429RetryEnabled: true,
                upstream429MaxRetries: 2,
              },
              recentInvocations: [],
            },
          ],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    }) as typeof fetch,
  );

  const response = await fetchUpstreamAccountActivity("today", {
    timeZone: "Asia/Shanghai",
    recentLimit: 6,
  });

  expect(response.accounts[0]?.firstByteAvgMs).toBe(0.0050063);
  expect(response.accounts[0]?.firstTokenAvgMs).toBe(2867.540251);
  expect(response.accounts[0]?.avgTotalMs).toBe(30581.6567545);
  expect(response.accounts[0]?.effectiveRoutingRule).toMatchObject({
    priorityTier: "no_new",
    allowCutIn: false,
    fastModeRewriteMode: "force_add",
    concurrencyLimit: 3,
    upstream429RetryEnabled: true,
    upstream429MaxRetries: 2,
  });
});
