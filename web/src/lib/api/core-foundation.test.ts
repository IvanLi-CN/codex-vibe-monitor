import { describe, expect, it } from "vitest";
import {
  acceptsRoutingStateVersion,
  compareRoutingStateVersion,
  normalizePoolRoutingSelectionAudit,
} from "./core-foundation";

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
