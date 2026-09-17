import { expect, it, vi } from "vitest";
import { fetchModelRoutingLive, fetchUpstreamAccountModelRoutingEvents } from "./api";

it("drops account-pool grouping metadata from routing snapshots and history", async () => {
  const displayName = "Aster";
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const path = typeof input === "string" ? input : input.toString();
      const payload = path.includes("model-routing-events")
        ? {
            items: [
              {
                id: "attempt:1",
                kind: "attempt",
                occurredAt: "2026-08-16T04:00:00.000Z",
                accountId: 11,
                accountDisplayName: displayName,
                accountGroupName: "unrelated-management-group",
                model: "gpt-5.5",
              },
            ],
            nextCursor: null,
          }
        : {
            generatedAt: "2026-08-16T04:00:00.000Z",
            groups: [
              {
                model: "gpt-5.5",
                accounts: [
                  {
                    accountId: 11,
                    accountDisplayName: displayName,
                    accountGroupName: "unrelated-management-group",
                    model: "gpt-5.5",
                    state: "available",
                    priority: "normal",
                    failureCount: 0,
                    lastSeenAt: "2026-08-16T04:00:00.000Z",
                  },
                ],
              },
            ],
            records: [
              {
                id: "attempt:1",
                kind: "attempt",
                occurredAt: "2026-08-16T04:00:00.000Z",
                accountId: 11,
                accountDisplayName: displayName,
                accountGroupName: "unrelated-management-group",
                model: "gpt-5.5",
              },
            ],
          };
      return new Response(JSON.stringify(payload), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }) as typeof fetch,
  );

  const [live, history] = await Promise.all([
    fetchModelRoutingLive({ window: "1h" }),
    fetchUpstreamAccountModelRoutingEvents(11, { model: "gpt-5.5" }),
  ]);

  expect(live.groups[0]?.accounts[0]).not.toHaveProperty("accountGroupName");
  expect(live.records[0]).not.toHaveProperty("accountGroupName");
  expect(history.items[0]).not.toHaveProperty("accountGroupName");
  expect(live.groups[0]?.accounts[0]).toHaveProperty("accountDisplayName", displayName);
  expect(live.records[0]).toHaveProperty("accountDisplayName", displayName);
  expect(history.items[0]).toHaveProperty("accountDisplayName", displayName);
});
