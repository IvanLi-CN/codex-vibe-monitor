import { afterEach, describe, expect, it, vi } from "vitest";
import {
  acceptsRoutingStateVersion,
  compareRoutingStateVersion,
  fetchSystemStatus,
  normalizePoolRoutingSelectionAudit,
} from "./core-foundation";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("fetchSystemStatus retention recovery compatibility", () => {
  it("keeps raw bytes unknown while the inventory is not ready", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              rawBodies: { count: 4, bytes: 0 },
              requestRawBodies: { count: 2, bytes: 0 },
              responseRawBodies: { count: 2, bytes: 0 },
              rawMetricsHealth: {
                state: "preparing",
                inventoryCursor: 12,
                physicalCoverage: "unknown",
              },
            }),
            { status: 200 },
          ),
      ),
    );

    const status = await fetchSystemStatus();

    expect(status.rawMetricsHealth).toMatchObject({
      state: "preparing",
      inventoryCursor: 12,
      physicalCoverage: "unknown",
    });
    expect(status.rawBodies.bytes).toBeNull();
    expect(status.requestRawBodies.bytes).toBeNull();
    expect(status.responseRawBodies.bytes).toBeNull();
  });

  it("preserves ready tracked bytes without assuming complete physical coverage", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              rawBodies: { count: 4, bytes: 17 },
              requestRawBodies: { count: 2, bytes: 11 },
              responseRawBodies: { count: 2, bytes: 9 },
              rawMetricsHealth: { state: "ready", inventoryCursor: 12 },
            }),
            { status: 200 },
          ),
      ),
    );

    const status = await fetchSystemStatus();

    expect(status.rawBodies.bytes).toBe(17);
    expect(status.requestRawBodies.bytes).toBe(11);
    expect(status.responseRawBodies.bytes).toBe(9);
    expect(status.rawMetricsHealth.physicalCoverage).toBe("unknown");
  });

  it("normalizes a missing recovery diagnostic to unknown for older backends", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () => new Response(JSON.stringify({ runtimePressureHealth: {} }), { status: 200 }),
      ),
    );

    const status = await fetchSystemStatus();

    expect(status.runtimePressureHealth?.retentionRecovery).toEqual({
      state: "unknown",
      stage: undefined,
      preparedCount: undefined,
      quarantinedCount: undefined,
      expiredBacklogCount: undefined,
      oldestBacklogAgeSecs: undefined,
      lastProgressAt: undefined,
      nextRetryAt: undefined,
      failureStage: undefined,
      failureFingerprint: undefined,
      deferReason: undefined,
      consecutiveFailureCount: undefined,
    });
  });

  it("keeps omitted recovery counters unknown when a partial diagnostic arrives", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              runtimePressureHealth: {
                retentionRecovery: { state: "recovering", preparedCount: 5 },
              },
            }),
            { status: 200 },
          ),
      ),
    );

    const status = await fetchSystemStatus();
    const recovery = status.runtimePressureHealth?.retentionRecovery;

    expect(recovery?.state).toBe("recovering");
    expect(recovery?.preparedCount).toBe(5);
    expect(recovery?.quarantinedCount).toBeUndefined();
    expect(recovery?.expiredBacklogCount).toBeUndefined();
  });

  it("normalizes explicitly unavailable recovery counters to unknown", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              runtimePressureHealth: {
                retentionRecovery: {
                  state: "unknown",
                  preparedCount: null,
                  quarantinedCount: null,
                  expiredBacklogCount: null,
                },
              },
            }),
            { status: 200 },
          ),
      ),
    );

    const recovery = (await fetchSystemStatus()).runtimePressureHealth?.retentionRecovery;

    expect(recovery?.preparedCount).toBeUndefined();
    expect(recovery?.quarantinedCount).toBeUndefined();
    expect(recovery?.expiredBacklogCount).toBeUndefined();
  });

  it("normalizes raw reference timing as an optional non-negative duration", async () => {
    const fetchStatus = async (rawReferenceCheckMs: unknown) => {
      vi.stubGlobal(
        "fetch",
        vi.fn(
          async () =>
            new Response(
              JSON.stringify({
                runtimePressureHealth: {
                  retentionWriteHealth: { state: "healthy", rawReferenceCheckMs },
                },
              }),
              { status: 200 },
            ),
        ),
      );
      return fetchSystemStatus();
    };

    expect(
      (await fetchStatus(-1)).runtimePressureHealth?.retentionWriteHealth?.rawReferenceCheckMs,
    ).toBeUndefined();
    expect(
      (await fetchStatus(null)).runtimePressureHealth?.retentionWriteHealth?.rawReferenceCheckMs,
    ).toBeUndefined();
    expect(
      (await fetchStatus(4.5)).runtimePressureHealth?.retentionWriteHealth?.rawReferenceCheckMs,
    ).toBe(4.5);
  });
});

describe("normalizePoolRoutingSelectionAudit", () => {
  it("preserves the optional recovery trigger", () => {
    const audit = normalizePoolRoutingSelectionAudit({
      selectedAccountId: 2918,
      selectedAccountName: "Ciii2",
      eligibleCandidateCount: 2,
      winnerReasonCode: "requestDrivenRecoveryAdmission",
      handoffAdmission: {
        decision: "admitted",
        phase: "verifying",
        verificationSuccessCount: 1,
        generation: 8,
        trigger: "modelRouteRecovery",
      },
      excludedCandidates: [],
    });

    expect(audit?.handoffAdmission?.trigger).toBe("modelRouteRecovery");
  });

  it("keeps historical admissions valid when trigger is absent", () => {
    const audit = normalizePoolRoutingSelectionAudit({
      selectedAccountId: 11,
      selectedAccountName: "Aster",
      eligibleCandidateCount: 1,
      winnerReasonCode: "onlyEligibleCandidate",
      handoffAdmission: {
        decision: "admitted",
        phase: "verifying",
        verificationSuccessCount: 0,
      },
      excludedCandidates: [],
    });

    expect(audit?.handoffAdmission).toEqual({
      decision: "admitted",
      phase: "verifying",
      verificationSuccessCount: 0,
    });
  });
});

describe("RoutingStateVersion", () => {
  const epoch = "2026-09-05T12:00:00.000000000Z";
  const older = { epoch, generation: "9" };
  const newer = { epoch, generation: "10" };
  const restarted = { epoch: "2026-09-05T12:00:01.000000000Z", generation: "1" };

  it("orders generations numerically within one process epoch", () => {
    expect(compareRoutingStateVersion(older, newer)).toBe(-1);
    expect(compareRoutingStateVersion(newer, older)).toBe(1);
    expect(acceptsRoutingStateVersion(newer, older, "live")).toBe(false);
    expect(acceptsRoutingStateVersion(older, newer, "live")).toBe(true);
  });

  it("only allows a cross-epoch snapshot or patch to replace the fence", () => {
    expect(acceptsRoutingStateVersion(newer, restarted, "live")).toBe(false);
    expect(acceptsRoutingStateVersion(newer, restarted, "snapshot")).toBe(true);
    expect(acceptsRoutingStateVersion(newer, restarted, "patch")).toBe(true);
  });

  it("allows a nullable patch without dropping an existing confirmation fence", () => {
    expect(acceptsRoutingStateVersion(newer, null, "patch")).toBe(true);
    expect(acceptsRoutingStateVersion(newer, null, "live")).toBe(false);
  });
});
