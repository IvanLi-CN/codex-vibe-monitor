import { describe, expect, it } from "vitest";
import { resolveDemoTopicPayload } from "./topic-payloads";

const requestUrl = "http://demo.invalid/events";

describe("demo topic payloads", () => {
  it("resolves every Live page subscription through the deterministic demo API", async () => {
    const [summary, forwardProxy, conversations, invocations] = await Promise.all([
      resolveDemoTopicPayload(
        { topic: "stats.summary.current", params: { window: "current", limit: "50" } },
        requestUrl,
      ),
      resolveDemoTopicPayload({ topic: "forward-proxy.live" }, requestUrl),
      resolveDemoTopicPayload(
        { topic: "prompt-cache.window", params: { limit: "50", detail: "full" } },
        requestUrl,
      ),
      resolveDemoTopicPayload({ topic: "invocations.window", params: { limit: "50" } }, requestUrl),
    ]);

    expect(summary).toMatchObject({ totalCount: expect.any(Number) });
    expect(forwardProxy).toMatchObject({ nodes: expect.any(Array) });
    expect(conversations).toMatchObject({ conversations: expect.any(Array) });
    expect(invocations).toMatchObject({ records: expect.any(Array), total: expect.any(Number) });
  });
});
