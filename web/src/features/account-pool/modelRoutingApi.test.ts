import { afterEach, describe, expect, it, vi } from "vitest";
import {
  fetchUpstreamAccountModelRouting,
  resetUpstreamAccountModelRouting,
} from "../../lib/api/core-upstream";

afterEach(() => vi.unstubAllGlobals());
describe("model routing API adapter", () => {
  it("loads catalog and policy unknown rows without claiming availability", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify([
          { model: "catalog-only", state: "unknown", lastSeenAt: "unknown" },
          { model: "policy-only", lastSeenAt: "unknown" },
          { model: "observed", state: "available", lastSeenAt: "2026-11-04T00:00:00Z" },
        ]),
        { headers: { "content-type": "application/json" } },
      ),
    );
    vi.stubGlobal("fetch", fetchMock);
    const states = await fetchUpstreamAccountModelRouting(42);
    expect(fetchMock.mock.calls[0][0]).toContain("/api/pool/upstream-accounts/42/model-routing");
    expect(states.map(({ model, state }) => [model, state])).toEqual([
      ["catalog-only", "unknown"],
      ["policy-only", "unknown"],
      ["observed", "available"],
    ]);
  });
  it("sends an exact single model reset and accepts the restored state", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          model: "limited-model",
          state: "available",
          failureCount: 0,
          lastSeenAt: "2026-11-04T00:00:00Z",
        }),
        { headers: { "content-type": "application/json" } },
      ),
    );
    vi.stubGlobal("fetch", fetchMock);
    expect((await resetUpstreamAccountModelRouting(42, "limited-model")).state).toBe("available");
    expect(fetchMock.mock.calls[0][0]).toContain("/42/model-routing/reset");
    expect(fetchMock.mock.calls[0][1]).toMatchObject({
      method: "POST",
      body: JSON.stringify({ model: "limited-model" }),
    });
  });
});
