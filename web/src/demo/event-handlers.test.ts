import { describe, expect, it } from "vitest";
import { handleDemoRequest } from "./handlers";
import { demoModel } from "./model";
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

  it("provides catalog, runtime, workload, and revision marker payloads to demo SSE topics", async () => {
    const [catalog, runtime, timeline, workload] = await Promise.all([
      resolveDemoTopicPayload({ topic: "system.managed-tasks.catalog" }, requestUrl),
      resolveDemoTopicPayload({ topic: "system.managed-tasks.runtime" }, requestUrl),
      resolveDemoTopicPayload(
        { topic: "system.managed-tasks.timeline", params: { schemaVersion: "2" } },
        requestUrl,
      ),
      resolveDemoTopicPayload(
        {
          topic: "system.managed-tasks.workload",
          params: { taskKey: "retention_archive", windowHours: "24", limit: "200" },
        },
        requestUrl,
      ),
    ]);

    expect(catalog).toEqual(
      expect.arrayContaining([expect.objectContaining({ taskKey: "retention_archive" })]),
    );
    expect(runtime).toMatchObject({
      activeRuns: expect.any(Array),
      queuedRuns: expect.any(Array),
      admissionWaits: expect.any(Array),
    });
    expect(timeline).toMatchObject({
      watermark: expect.any(Number),
      observedAt: expect.any(String),
    });
    expect(workload).toMatchObject({ samples: expect.any(Array), coverage: expect.any(String) });
    expect(timeline).not.toHaveProperty("segments");
    expect(timeline).not.toHaveProperty("coverage");
  });

  it("serves the dense task timeline through 500-row fixed-window pages", async () => {
    const from = new Date(Date.now() - 12 * 60 * 60 * 1000).toISOString();
    const to = new Date().toISOString();
    const ids = new Set<string>();
    let cursor: string | null = null;
    let watermark: number | null = null;
    let pageCount = 0;
    do {
      const query = new URLSearchParams({ from, to, limit: "500" });
      if (cursor) query.set("cursor", cursor);
      const response = await handleDemoRequest(
        new Request(`http://demo.invalid/api/system/managed-tasks/timeline?${query.toString()}`),
      );
      const page = (await response.json()) as {
        watermark: number;
        segments: Array<{ segmentId: string }>;
        nextCursor: string | null;
        windowStart: string;
        windowEnd: string;
      };
      watermark ??= page.watermark;
      expect(page.watermark).toBe(watermark);
      expect(page.windowStart).toBe(from);
      expect(page.windowEnd).toBe(to);
      expect(page.segments.length).toBeLessThanOrEqual(500);
      for (const segment of page.segments) ids.add(segment.segmentId);
      cursor = page.nextCursor;
      pageCount += 1;
    } while (cursor);

    expect(ids.size).toBe(13_120);
    expect(pageCount).toBe(27);
  });

  it("serves the dense pressure scene with fixed deferral groups and open waits", async () => {
    demoModel.setScene("task-timeline-pressure-dense");
    const from = new Date(Date.now() - 12 * 60 * 60 * 1000).toISOString();
    const to = new Date().toISOString();
    const segments: Array<{
      kind: string;
      reason?: string | null;
      finishedAt?: string | null;
    }> = [];
    let cursor: string | null = null;
    do {
      const query = new URLSearchParams({ from, to, limit: "500" });
      if (cursor) query.set("cursor", cursor);
      const response = await handleDemoRequest(
        new Request(`http://demo.invalid/api/system/managed-tasks/timeline?${query.toString()}`),
      );
      const page = (await response.json()) as {
        segments: typeof segments;
        nextCursor: string | null;
        watermark: number;
      };
      segments.push(...page.segments);
      cursor = page.nextCursor;
      expect(page.watermark).toBe(13_120);
    } while (cursor);

    const deferrals = segments.filter((segment) => segment.kind === "deferral");
    expect(segments).toHaveLength(13_120);
    expect(deferrals).toHaveLength(6_000);
    expect(new Set(deferrals.map((segment) => segment.reason))).toEqual(
      new Set(["resource_busy", "pressure_cooldown"]),
    );
    expect(deferrals.filter((segment) => segment.finishedAt == null)).toHaveLength(4);
    demoModel.setScene("operational");
  });

  it("resets a timeline cursor when the demo scene changes", async () => {
    demoModel.setScene("task-timeline-pressure-dense");
    const from = new Date(Date.now() - 12 * 60 * 60 * 1_000).toISOString();
    const to = new Date().toISOString();
    const firstPage = await handleDemoRequest(
      new Request(
        `http://demo.invalid/api/system/managed-tasks/timeline?from=${from}&to=${to}&limit=500`,
      ),
    );
    const firstPayload = (await firstPage.json()) as { nextCursor: string | null };
    expect(firstPayload.nextCursor).not.toBeNull();

    demoModel.setScene("operational");
    const continuation = await handleDemoRequest(
      new Request(
        `http://demo.invalid/api/system/managed-tasks/timeline?cursor=${firstPayload.nextCursor}`,
      ),
    );
    await expect(continuation.json()).resolves.toMatchObject({
      segments: [],
      nextCursor: null,
      resetRequired: true,
    });
    demoModel.setScene("operational");
  });

  it("resets a timeline cursor when a scene is regenerated after a round trip", async () => {
    demoModel.setScene("task-timeline-pressure-dense");
    const from = new Date(Date.now() - 12 * 60 * 60 * 1_000).toISOString();
    const to = new Date().toISOString();
    const firstPage = await handleDemoRequest(
      new Request(
        `http://demo.invalid/api/system/managed-tasks/timeline?from=${from}&to=${to}&limit=500`,
      ),
    );
    const firstPayload = (await firstPage.json()) as { nextCursor: string | null };
    expect(firstPayload.nextCursor).not.toBeNull();

    demoModel.setScene("operational");
    demoModel.setScene("task-timeline-pressure-dense");
    const continuation = await handleDemoRequest(
      new Request(
        `http://demo.invalid/api/system/managed-tasks/timeline?cursor=${firstPayload.nextCursor}`,
      ),
    );
    await expect(continuation.json()).resolves.toMatchObject({
      segments: [],
      nextCursor: null,
      resetRequired: true,
    });
    demoModel.setScene("operational");
  });

  it("keeps unversioned timeline demo subscriptions on the bounded v1 contract", async () => {
    await expect(
      resolveDemoTopicPayload({ topic: "system.managed-tasks.timeline" }, requestUrl),
    ).rejects.toThrow("managed task timeline exceeds the bounded SSE snapshot capacity");
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
        accounts: Array<{ accountId: number; state: string }>;
      }>;
      records: Array<{ model: string }>;
    };

    expect(payload.groups).toEqual([
      expect.objectContaining({
        model: "gpt-5.4-mini",
        accounts: expect.arrayContaining([
          expect.objectContaining({
            accountId: 102,
            state: "cooling_down",
          }),
        ]),
      }),
    ]);
    expect(payload.records).toHaveLength(1);
    expect(payload.records[0]).toMatchObject({ model: "gpt-5.4-mini" });
  });
});
