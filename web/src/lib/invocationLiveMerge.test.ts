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

  it("does not backfill stale failure metadata into a recovered terminal success", () => {
    const runtimeFailure = createRecord({
      id: 3,
      invokeId: "invoke-3",
      occurredAt: "2026-03-10T02:32:00Z",
      status: "running",
      errorMessage: "upstream timeout",
      failureKind: "upstream_timeout",
      poolAttemptTerminalReason: "budget_exhausted_final",
      upstreamErrorCode: "rate_limit",
      upstreamErrorMessage: "quota exhausted",
      isActionable: true,
      proxyDisplayName: "Proxy Live",
    });
    const finalSuccess = createRecord({
      id: 3,
      invokeId: "invoke-3",
      occurredAt: "2026-03-10T02:32:00Z",
      status: "success",
      failureClass: "none",
      proxyDisplayName: "Proxy Final",
      requestedServiceTier: "flex",
    });

    const merged = mergeInvocationRecordCollections([runtimeFailure], [finalSuccess]);

    expect(merged).toHaveLength(1);
    expect(merged[0]?.status).toBe("success");
    expect(merged[0]?.proxyDisplayName).toBe("Proxy Final");
    expect(merged[0]?.requestedServiceTier).toBe("flex");
    expect(merged[0]?.errorMessage).toBeUndefined();
    expect(merged[0]?.failureKind).toBeUndefined();
    expect(merged[0]?.poolAttemptTerminalReason).toBeUndefined();
    expect(merged[0]?.upstreamErrorCode).toBeUndefined();
    expect(merged[0]?.upstreamErrorMessage).toBeUndefined();
    expect(merged[0]?.isActionable).toBeUndefined();
  });

  it("still backfills failure metadata when the preferred terminal record is failed", () => {
    const runtimeFailure = createRecord({
      id: 4,
      invokeId: "invoke-4",
      occurredAt: "2026-03-10T02:33:00Z",
      status: "running",
      errorMessage: "upstream timeout",
      failureKind: "upstream_timeout",
      poolAttemptTerminalReason: "budget_exhausted_final",
      upstreamErrorCode: "rate_limit",
      upstreamErrorMessage: "quota exhausted",
      isActionable: true,
    });
    const finalFailure = createRecord({
      id: 4,
      invokeId: "invoke-4",
      occurredAt: "2026-03-10T02:33:00Z",
      status: "http_429",
      failureClass: "service_failure",
      proxyDisplayName: "Proxy Final",
    });

    const merged = mergeInvocationRecordCollections([runtimeFailure], [finalFailure]);

    expect(merged).toHaveLength(1);
    expect(merged[0]?.status).toBe("http_429");
    expect(merged[0]?.errorMessage).toBe("upstream timeout");
    expect(merged[0]?.failureKind).toBe("upstream_timeout");
    expect(merged[0]?.poolAttemptTerminalReason).toBe("budget_exhausted_final");
    expect(merged[0]?.upstreamErrorCode).toBe("rate_limit");
    expect(merged[0]?.upstreamErrorMessage).toBe("quota exhausted");
    expect(merged[0]?.isActionable).toBe(true);
  });
});
