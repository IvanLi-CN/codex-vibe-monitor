import { describe, expect, it } from "vitest";
import { normalizePoolRoutingSelectionAudit } from "./core-foundation";

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
