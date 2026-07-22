import { describe, expect, it } from "vitest";
import {
  buildAppliedInvocationFilters,
  buildInvocationSuggestionsQuery,
  createDefaultInvocationRecordsDraft,
} from "./invocationRecords";

describe("buildAppliedInvocationFilters", () => {
  it("rejects fractional token filters before sending the request", () => {
    const draft = {
      ...createDefaultInvocationRecordsDraft(),
      minTotalTokens: "1.5",
    };

    expect(() => buildAppliedInvocationFilters(draft)).toThrow(
      "Total tokens range must use whole numbers",
    );
  });

  it("treats minute-precision customTo as inclusive-of-minute for exclusive upper bounds", () => {
    const draft = {
      ...createDefaultInvocationRecordsDraft(),
      rangePreset: "custom" as const,
      customFrom: "2026-03-10T10:00",
      customTo: "2026-03-10T10:32",
    };

    const filters = buildAppliedInvocationFilters(draft);
    expect(filters.from).toBeDefined();
    expect(filters.to).toBeDefined();

    const expected = new Date("2026-03-10T10:32").getTime() + 60_000;
    expect(new Date(filters.to as string).getTime()).toBe(expected);
  });

  it("keeps second-precision customTo untouched", () => {
    const draft = {
      ...createDefaultInvocationRecordsDraft(),
      rangePreset: "custom" as const,
      customFrom: "2026-03-10T10:00:00",
      customTo: "2026-03-10T10:32:45",
    };

    const filters = buildAppliedInvocationFilters(draft);
    const expected = new Date("2026-03-10T10:32:45").getTime();
    expect(new Date(filters.to as string).getTime()).toBe(expected);
  });

  it("builds suggestion queries from the full draft filters and current snapshot", () => {
    const draft = {
      ...createDefaultInvocationRecordsDraft(),
      rangePreset: "custom" as const,
      customFrom: "2026-03-10T10:00:00",
      customTo: "2026-03-10T10:32:45",
      status: " failed ",
      models: [" gpt-5.4 ", " gpt-5.5 "],
      modelTarget: "response" as const,
      modelRerouted: "rerouted" as const,
      endpoint: " /v1/responses ",
      failureClass: " service_failure ",
      failureKind: " http_502 ",
      promptCacheKey: " cache-key ",
      reasoningEfforts: [" high ", " medium "],
      requesterIp: " 127.0.0.1 ",
      keyword: " retry ",
      minTotalTokens: "10",
      maxTotalTokens: "20",
      minTotalMs: "1.5",
      maxTotalMs: "2.5",
    };

    const query = buildInvocationSuggestionsQuery(draft, 99);

    expect(query.snapshotId).toBe(99);
    expect(query.status).toBe("failed");
    expect(query.models).toEqual(["gpt-5.4", "gpt-5.5"]);
    expect(query.modelTarget).toBe("response");
    expect(query.modelRerouted).toBe(true);
    expect(query.endpoint).toBe("/v1/responses");
    expect(query.failureClass).toBe("service_failure");
    expect(query.failureKind).toBe("http_502");
    expect(query.promptCacheKey).toBe("cache-key");
    expect(query.reasoningEfforts).toEqual(["high", "medium"]);
    expect(query.requesterIp).toBe("127.0.0.1");
    expect(query.keyword).toBe("retry");
    expect(query.minTotalTokens).toBe(10);
    expect(query.maxTotalTokens).toBe(20);
    expect(query.minTotalMs).toBe(1.5);
    expect(query.maxTotalMs).toBe(2.5);
    expect(query.suggestField).toBeUndefined();
    expect(query.suggestQuery).toBeUndefined();
    expect(query.from).toBeDefined();
    expect(query.to).toBeDefined();
  });

  it("maps the not-rerouted draft state to an explicit false query value", () => {
    const draft = {
      ...createDefaultInvocationRecordsDraft(),
      models: ["gpt-5.4"],
      modelRerouted: "notRerouted" as const,
    };

    const query = buildAppliedInvocationFilters(draft);

    expect(query.modelRerouted).toBe(false);
  });

  it("includes the active suggestion field and server-side search text when provided", () => {
    const draft = {
      ...createDefaultInvocationRecordsDraft(),
      models: ["gpt-5.4-mini"],
    };

    const query = buildInvocationSuggestionsQuery(
      draft,
      42,
      "requestModel",
      new Date(),
      "gpt-5.4-mini",
    );

    expect(query.snapshotId).toBe(42);
    expect(query.suggestField).toBe("requestModel");
    expect(query.suggestQuery).toBe("gpt-5.4-mini");
    expect(query.models).toEqual(["gpt-5.4-mini"]);
    expect(query.modelTarget).toBe("request");
  });

  it("rejects reasoning-effort filters without a selected model", () => {
    const draft = {
      ...createDefaultInvocationRecordsDraft(),
      reasoningEfforts: ["high"],
    };

    expect(() => buildAppliedInvocationFilters(draft)).toThrow(
      "Model filter requires at least one model",
    );
  });

  it("normalizes invokeId and attemptId across applied filters and suggestion queries", () => {
    const draft = {
      ...createDefaultInvocationRecordsDraft(),
      invokeId: " invoke-123 ",
      attemptId: " 4V7MYPJG ",
    };

    expect(buildAppliedInvocationFilters(draft).invokeId).toBe("invoke-123");
    expect(buildAppliedInvocationFilters(draft).attemptId).toBe("4V7MYPJG");
    expect(buildInvocationSuggestionsQuery(draft, 42).invokeId).toBe("invoke-123");
    expect(buildInvocationSuggestionsQuery(draft, 42).attemptId).toBe("4V7MYPJG");
  });

  it("uses upstreamAccountId for exact account filters while keeping the display label in draft", () => {
    const draft = {
      ...createDefaultInvocationRecordsDraft(),
      upstreamAccount: "Pool Alpha (#42)",
      upstreamAccountId: "42",
    };

    expect(buildAppliedInvocationFilters(draft).upstreamAccountId).toBe(42);
    expect(buildInvocationSuggestionsQuery(draft, 42).upstreamAccountId).toBe(42);
  });

  it("tolerates invalid draft values when building suggestion queries", () => {
    const draft = {
      ...createDefaultInvocationRecordsDraft(),
      rangePreset: "custom" as const,
      customFrom: "2026-03-10T10:",
      customTo: "not-a-date",
      minTotalTokens: "1.5",
      maxTotalTokens: "abc",
    };

    expect(() => buildInvocationSuggestionsQuery(draft, 42)).not.toThrow();

    const query = buildInvocationSuggestionsQuery(draft, 42);
    expect(query.snapshotId).toBe(42);
    expect(query.from).toBeUndefined();
    expect(query.to).toBeUndefined();
    expect(query.minTotalTokens).toBeUndefined();
    expect(query.maxTotalTokens).toBeUndefined();
  });
});
