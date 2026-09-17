import { expect, it, vi } from "vitest";
import {
  createForwardProxyNodesLatencyTestEventSource,
  normalizeForwardProxyLatencyTestStreamEvent,
  refreshForwardProxySubscriptions,
} from "./api";

it("normalizes latency stream events with successful samples", () => {
  const event = normalizeForwardProxyLatencyTestStreamEvent({
    kind: "progress",
    node: {
      key: "node-a",
      displayName: "Node A",
      round: 2,
      totalRounds: 5,
      completedRounds: 2,
      successCount: 3,
      attemptCount: 4,
      averageLatencyMs: 151.4,
      egressIp: { ok: true, latencyMs: 120, ip: "203.0.113.10" },
      oauthUpstream: { ok: true, latencyMs: 183, httpStatus: 401 },
      codexResponses: { ok: true, latencyMs: 151, httpStatus: 405 },
      allTargetsOk: true,
      failedTargets: [],
      done: false,
      timedOut: false,
      message: "151 ms",
    },
  });

  expect(event?.kind).toBe("progress");
  expect(event?.node.averageLatencyMs).toBe(151.4);
  expect(event?.node.egressIp.ip).toBe("203.0.113.10");
  expect(event?.node.oauthUpstream.httpStatus).toBe(401);
  expect(event?.node.codexResponses.httpStatus).toBe(405);
  expect(event?.node.allTargetsOk).toBe(true);
});
it("normalizes failed Codex responses probes as abnormal", () => {
  const event = normalizeForwardProxyLatencyTestStreamEvent({
    kind: "completed",
    node: {
      key: "node-b",
      displayName: "Node B",
      round: 1,
      totalRounds: 5,
      completedRounds: 1,
      successCount: 2,
      attemptCount: 3,
      averageLatencyMs: 131,
      egressIp: { ok: true, latencyMs: 120, ip: "203.0.113.11" },
      oauthUpstream: { ok: true, latencyMs: 142, httpStatus: 401 },
      codexResponses: { ok: false, error: "responses timeout" },
      allTargetsOk: false,
      failedTargets: ["codexResponses"],
      done: true,
      timedOut: true,
      message: "failed targets: codexResponses",
    },
  });

  expect(event?.kind).toBe("completed");
  expect(event?.node.averageLatencyMs).toBe(131);
  expect(event?.node.allTargetsOk).toBe(false);
  expect(event?.node.failedTargets).toEqual(["codexResponses"]);
  expect(event?.node.codexResponses.error).toBe("responses timeout");
});
it("normalizes forced subscription refresh response", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      return new Response(
        JSON.stringify({
          forwardProxy: {
            proxyUrls: ["socks5://127.0.0.1:1080"],
            subscriptionUrls: ["https://example.com/sub"],
            subscriptionUpdateIntervalSecs: 900,
            nodes: [],
          },
          subscriptionCount: 1,
          addedNodeCount: 2,
          refreshedAt: "2026-05-19T06:00:00Z",
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    }) as typeof fetch,
  );

  const response = await refreshForwardProxySubscriptions();
  expect(response.forwardProxy.subscriptionUpdateIntervalSecs).toBe(900);
  expect(response.subscriptionCount).toBe(1);
  expect(response.addedNodeCount).toBe(2);
});
it("creates breadth-first batch test event source URL with repeated keys", () => {
  class MockEventSource {
    static latestUrl = "";
    readonly url: string;
    constructor(url: string | URL) {
      this.url = url.toString();
      MockEventSource.latestUrl = this.url;
    }
    close() {}
  }
  vi.stubGlobal("EventSource", MockEventSource as unknown as typeof EventSource);

  createForwardProxyNodesLatencyTestEventSource(["node-a", "node-b"]);
  const url = new URL(MockEventSource.latestUrl, "http://localhost");
  expect(url.pathname).toBe("/api/settings/forward-proxy/nodes/test-stream");
  expect(url.searchParams.getAll("key")).toEqual(["node-a", "node-b"]);
});
