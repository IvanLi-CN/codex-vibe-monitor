import { describe, expect, it } from "vitest";
import type { ApiInvocation } from "./api";
import { mergeInvocationWindowRecords } from "./invocationRecordsLive";

function createRecord(
  overrides: Partial<ApiInvocation> & {
    id: number;
    invokeId: string;
    occurredAt: string;
  },
): ApiInvocation {
  const { id, invokeId, occurredAt, ...rest } = overrides;
  return {
    id,
    invokeId,
    occurredAt,
    createdAt: rest.createdAt ?? occurredAt,
    status: rest.status ?? "success",
    model: rest.model ?? "gpt-5.4",
    ...rest,
  };
}

describe("invocationRecordsLive", () => {
  it("keeps null metric values at the end for ascending live windows", () => {
    const current = [
      createRecord({
        id: 1,
        invokeId: "invoke-null",
        occurredAt: "2026-03-10T00:00:00Z",
        totalTokens: null,
      }),
      createRecord({
        id: 2,
        invokeId: "invoke-valued",
        occurredAt: "2026-03-10T00:01:00Z",
        totalTokens: 200,
      }),
    ];

    const merged = mergeInvocationWindowRecords(current, [], {
      sortBy: "totalTokens",
      sortOrder: "asc",
      limit: 10,
    });

    expect(merged.map((record) => record.invokeId)).toEqual(["invoke-valued", "invoke-null"]);
  });

  it("keeps null metric values at the end for descending live windows", () => {
    const current = [
      createRecord({
        id: 1,
        invokeId: "invoke-null",
        occurredAt: "2026-03-10T00:00:00Z",
        totalTokens: null,
      }),
      createRecord({
        id: 2,
        invokeId: "invoke-valued",
        occurredAt: "2026-03-10T00:01:00Z",
        totalTokens: 200,
      }),
    ];

    const merged = mergeInvocationWindowRecords(current, [], {
      sortBy: "totalTokens",
      sortOrder: "desc",
      limit: 10,
    });

    expect(merged.map((record) => record.invokeId)).toEqual(["invoke-valued", "invoke-null"]);
  });

  it("matches backend DESC occurredAt tie-breaks for ascending non-time sorts", () => {
    const current = [
      createRecord({
        id: 1,
        invokeId: "invoke-earlier",
        occurredAt: "2026-03-10T00:00:00Z",
        totalTokens: 200,
      }),
      createRecord({
        id: 2,
        invokeId: "invoke-later",
        occurredAt: "2026-03-10T00:01:00Z",
        totalTokens: 200,
      }),
    ];

    const merged = mergeInvocationWindowRecords(current, [], {
      sortBy: "totalTokens",
      sortOrder: "asc",
      limit: 10,
    });

    expect(merged.map((record) => record.invokeId)).toEqual(["invoke-later", "invoke-earlier"]);
  });

  it("keeps a terminal record when a stale running update arrives later", () => {
    const current = [
      createRecord({
        id: 20,
        invokeId: "invoke-terminal",
        occurredAt: "2026-03-10T00:04:00Z",
        status: "success",
        totalTokens: 18,
        cost: 0.0025,
        tTotalMs: 2400,
      }),
    ];

    const merged = mergeInvocationWindowRecords(
      current,
      [
        createRecord({
          id: -20,
          invokeId: "invoke-terminal",
          occurredAt: "2026-03-10T00:04:00Z",
          status: "running",
        }),
      ],
      {
        sortBy: "occurredAt",
        sortOrder: "desc",
        limit: 10,
      },
    );

    expect(merged).toHaveLength(1);
    expect(merged[0]?.status).toBe("success");
    expect(merged[0]?.totalTokens).toBe(18);
  });

  it("keeps transient records out of an attempt-scoped merge until the server resolves them", () => {
    const current = [
      createRecord({
        id: 30,
        invokeId: "invoke-attempt-filter",
        occurredAt: "2026-03-10T00:05:00Z",
      }),
    ];

    const merged = mergeInvocationWindowRecords(current, [], {
      filters: { attemptId: "4V7MYPJG" },
      sortBy: "occurredAt",
      sortOrder: "desc",
      limit: 10,
    });

    expect(merged).toHaveLength(0);
  });
});
