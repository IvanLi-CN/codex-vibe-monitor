import { describe, expect, it } from "vitest";
import { resolveDemoTopicPayload } from "./topic-payloads";

const requestUrl = "http://demo.invalid/events";

describe("demo topic payloads", () => {
  it("resolves every Live page subscription through the deterministic demo API", async () => {
    const [summary, forwardProxy, modelRouting, conversations, invocations] = await Promise.all([
      resolveDemoTopicPayload(
        { topic: "stats.summary.current", params: { window: "current", limit: "50" } },
        requestUrl,
      ),
      resolveDemoTopicPayload({ topic: "forward-proxy.live" }, requestUrl),
      resolveDemoTopicPayload(
        { topic: "pool.model-routing-live", params: { window: "1h", limit: "100" } },
        requestUrl,
      ),
      resolveDemoTopicPayload(
        { topic: "prompt-cache.window", params: { limit: "50", detail: "full" } },
        requestUrl,
      ),
      resolveDemoTopicPayload({ topic: "invocations.window", params: { limit: "50" } }, requestUrl),
    ]);

    expect(summary).toMatchObject({ totalCount: expect.any(Number) });
    expect(forwardProxy).toMatchObject({ nodes: expect.any(Array) });
    expect(modelRouting).toMatchObject({ groups: expect.any(Array), records: expect.any(Array) });
    expect(conversations).toMatchObject({ conversations: expect.any(Array) });
    expect(invocations).toMatchObject({ records: expect.any(Array), total: expect.any(Number) });
  });

  it("keeps model routing subscription filters in the demo snapshot", async () => {
    const payload = (await resolveDemoTopicPayload(
      {
        topic: "pool.model-routing-live",
        params: {
          window: "1h",
          model: "gpt-5.4-mini",
          state: "cooling_down",
          limit: "1",
        },
      },
      requestUrl,
    )) as {
      groups: Array<{
        model: string;
        accounts: Array<{ accountDisplayName: string; state: string }>;
      }>;
      records: Array<{ model: string }>;
    };

    expect(payload.groups).toEqual([
      expect.objectContaining({
        model: "gpt-5.4-mini",
        accounts: expect.arrayContaining([
          expect.objectContaining({ accountDisplayName: "prod-api-key-a", state: "cooling_down" }),
        ]),
      }),
    ]);
    expect(payload.records).toHaveLength(1);
    expect(payload.records[0]).toMatchObject({ model: "gpt-5.4-mini" });
  });
});
