import { expect, it, vi } from "vitest";
import { fetchDashboardActivity, fetchDashboardActivityRecent } from "./api";

it("normalizes the unified dashboard activity snapshot", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      return new Response(
        JSON.stringify({
          range: "today",
          rangeStart: "2026-07-05T00:00:00Z",
          rangeEnd: "2026-07-05T12:00:00Z",
          snapshotId: 1783233600000,
          rateWindow: {
            start: "2026-07-05T11:59:00Z",
            end: "2026-07-05T12:00:00Z",
            windowMinutes: 1,
            mode: "rolling_60s_live_mean",
          },
          summary: {
            stats: {
              totalCount: 3,
              successCount: 1,
              failureCount: 0,
              totalCost: 0.6,
              totalTokens: 6000,
              inProgressConversationCount: 2,
              inProgressRetryConversationCount: 1,
              inProgressAvgWaitMs: 250,
              nonSuccessCost: 0,
              nonSuccessTokens: 0,
            },
            tokensPerMinute: 1200,
            spendRate: 0.12,
            currentFirstTokenAvgMs: 1500,
            currentAvgTotalMs: 2400,
          },
          accounts: [
            {
              accountKey: "unassigned",
              upstreamAccountId: null,
              displayName: "未分配上游账号",
              isUnassigned: true,
              requestCount: 1,
              successCount: 0,
              failureCount: 0,
              nonSuccessCount: 0,
              totalTokens: 3000,
              successTokens: 0,
              nonSuccessTokens: 0,
              failureTokens: 0,
              failureCost: 0,
              totalCost: 0.3,
              tokensPerMinute: 600,
              spendRate: 0.06,
              recentInvocations: [],
            },
          ],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    }) as typeof fetch,
  );

  const response = await fetchDashboardActivity("today", {
    timeZone: "Asia/Shanghai",
    includeAccounts: true,
    recentLimit: 4,
  });

  expect(response.summary.stats.inProgressConversationCount).toBe(2);
  expect(response.summary.tokensPerMinute).toBe(1200);
  expect(response.rateWindow.mode).toBe("rolling_60s_live_mean");
  expect(response.accounts?.[0]?.accountKey).toBe("unassigned");
  expect(response.accounts?.[0]?.upstreamAccountId).toBeNull();
  expect(response.accounts?.[0]?.isUnassigned).toBe(true);
});
it("serializes progressive options and normalizes the recent batch", async () => {
  const requestedUrls: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      requestedUrls.push(String(input));
      return new Response(
        JSON.stringify({
          rangeStart: "2026-07-05T00:00:00.000Z",
          rangeEnd: "2026-07-05T12:00:00.000Z",
          snapshotId: 1783233600000,
          accounts: [
            {
              accountKey: "upstream:42",
              recentInvocations: [
                {
                  id: 1,
                  invokeId: "recent-1",
                  occurredAt: "2026-07-05T11:59:00Z",
                  status: "success",
                  totalTokens: 12,
                },
              ],
            },
          ],
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    }) as typeof fetch,
  );

  await fetchDashboardActivity("today", {
    includeAccounts: true,
    includeRecent: false,
  });
  const response = await fetchDashboardActivityRecent({
    rangeStart: "2026-07-05T00:00:00.000Z",
    rangeEnd: "2026-07-05T12:00:00.000Z",
    snapshotId: 1783233600000,
    recentLimit: 4,
  });

  expect(requestedUrls[0]).toContain("includeRecent=false");
  expect(requestedUrls[1]).toContain("/api/stats/dashboard-activity/recent?");
  expect(requestedUrls[1]).toContain("snapshotId=1783233600000");
  expect(response.accounts[0]?.recentInvocations[0]?.invokeId).toBe("recent-1");
});
