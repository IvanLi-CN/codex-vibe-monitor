import { describe, expect, it } from "vitest";
import { translations } from "./translations";

describe("translations", () => {
  it("localizes chart non-success labels per locale", () => {
    expect(translations.en["chart.nonSuccess"]).toBe("Non-success");
    expect(translations.zh["chart.nonSuccess"]).toBe("非成功");
  });

  it("keeps recent account event protocol values localized in both locales", () => {
    const keys = [
      "accountPool.upstreamAccounts.latestAction.actions.route_cooldown_started",
      "accountPool.upstreamAccounts.latestAction.actions.model_route_cooldown",
      "accountPool.upstreamAccounts.latestAction.sources.call",
      "accountPool.upstreamAccounts.latestAction.sources.sync_maintenance",
      "accountPool.upstreamAccounts.latestAction.reasons.egress_throttled",
      "accountPool.upstreamAccounts.latestAction.reasons.upstream_rejected",
      "accountPool.upstreamAccounts.latestAction.reasons.pool_assigned_account_blocked",
      "accountPool.upstreamAccounts.latestAction.reasons.upstream_http_429_quota_exhausted",
      "accountPool.upstreamAccounts.modelRouting.states.degraded",
      "accountPool.upstreamAccounts.modelRouting.states.cooling_down",
      "accountPool.upstreamAccounts.modelRouting.priorities.demoted",
      "accountPool.upstreamAccounts.modelRouting.priorities.excluded",
      "accountPool.upstreamAccounts.modelRouting.failureKinds.model",
      "accountPool.upstreamAccounts.modelRouting.failureKinds.model_unavailable",
      "accountPool.upstreamAccounts.modelRouting.failureKinds.model_quota",
      "accountPool.upstreamAccounts.recentActions.blockedBinding.encryptedSessionOwner",
      "accountPool.upstreamAccounts.recentActions.blockedBinding.explicitAccount",
      "accountPool.upstreamAccounts.recentActions.blockedBinding.openConversations",
      "accountPool.upstreamAccounts.recentActions.historicalEventUnlinkedAttempt",
    ] as const;

    for (const key of keys) {
      expect(translations.en[key]).toBeTruthy();
      expect(translations.zh[key]).toBeTruthy();
      expect(translations.en[key]).not.toBe(key);
      expect(translations.zh[key]).not.toBe(key);
    }
  });
});
