import { describe, expect, it } from "vitest";
import type { ApiInvocation } from "./api";
import {
  choosePreferredInvocationRecord,
  mergeInvocationRecordCollections,
} from "./invocationLiveMerge";

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
    status: rest.status ?? "completed",
    totalTokens: rest.totalTokens ?? 100,
    cost: rest.cost ?? 0.01,
    ...rest,
  };
}

describe("invocationLiveMerge", () => {
  it("lets later collections override equal-completeness records", () => {
    const live = createRecord({
      id: 1,
      invokeId: "invoke-1",
      occurredAt: "2026-03-10T02:30:00Z",
      proxyDisplayName: "Proxy Live",
      requestedServiceTier: "auto",
    });
    const authoritative = createRecord({
      id: 1,
      invokeId: "invoke-1",
      occurredAt: "2026-03-10T02:30:00Z",
      proxyDisplayName: "Proxy Final",
      requestedServiceTier: "flex",
    });

    const merged = mergeInvocationRecordCollections([live], [authoritative]);

    expect(merged).toHaveLength(1);
    expect(merged[0]?.proxyDisplayName).toBe("Proxy Final");
    expect(merged[0]?.requestedServiceTier).toBe("flex");
  });

  it("keeps the current record on exact ties for direct preference checks", () => {
    const authoritative = createRecord({
      id: 2,
      invokeId: "invoke-2",
      occurredAt: "2026-03-10T02:31:00Z",
      proxyDisplayName: "Proxy Final",
      requestedServiceTier: "flex",
    });
    const live = createRecord({
      id: 2,
      invokeId: "invoke-2",
      occurredAt: "2026-03-10T02:31:00Z",
      proxyDisplayName: "Proxy Live",
      requestedServiceTier: "auto",
    });

    expect(choosePreferredInvocationRecord(authoritative, live)).toBe(authoritative);
  });
});
