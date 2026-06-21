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

    expect(merged.map((record) => record.invokeId)).toEqual([
      "invoke-valued",
      "invoke-null",
    ]);
  });

  it("keeps ascending tie-breaks stable for non-occurredAt sorts", () => {
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

    expect(merged.map((record) => record.invokeId)).toEqual([
      "invoke-earlier",
      "invoke-later",
    ]);
  });
});
