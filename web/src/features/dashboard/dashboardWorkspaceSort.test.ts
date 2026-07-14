import { describe, expect, it } from "vitest";
import type { UpstreamAccountActivityAccount } from "../../lib/api";
import type { DashboardWorkingConversationCardModel } from "../../lib/dashboardWorkingConversations";
import {
  compareDashboardConversationCards,
  compareDashboardUpstreamAccounts,
  nextDashboardWorkspaceSort,
} from "./dashboardWorkspaceSort";

describe("dashboard workspace sorting", () => {
  it("cycles through the four modes", () => {
    expect(nextDashboardWorkspaceSort("createdAt")).toBe("lastInvocation");
    expect(nextDashboardWorkspaceSort("lastInvocation")).toBe("cost");
    expect(nextDashboardWorkspaceSort("cost")).toBe("tokens");
    expect(nextDashboardWorkspaceSort("tokens")).toBe("createdAt");
  });

  it("sorts conversations by descending times and ascending metrics", () => {
    const make = (
      key: string,
      created: number | null,
      invoked: number | null,
      cost: number,
      tokens: number,
    ) =>
      ({
        promptCacheKey: key,
        createdAtEpoch: created,
        totalCost: cost,
        totalTokens: tokens,
        currentInvocation: { occurredAtEpoch: invoked },
      }) as DashboardWorkingConversationCardModel;
    const older = make("b", 10, 20, 2, 20);
    const newer = make("a", 30, 40, 1, 10);
    expect(
      [older, newer].sort((a, b) => compareDashboardConversationCards(a, b, "createdAt")),
    ).toEqual([newer, older]);
    expect([older, newer].sort((a, b) => compareDashboardConversationCards(a, b, "cost"))).toEqual([
      newer,
      older,
    ]);
  });

  it("uses account aggregate timestamps and places missing values last", () => {
    const make = (accountKey: string, createdAt: string | null, invokedAt: string | null) =>
      ({
        accountKey,
        latestConversationCreatedAt: createdAt,
        lastInvocationAt: invokedAt,
        totalCost: 0,
        totalTokens: 0,
      }) as UpstreamAccountActivityAccount;
    const present = make("b", "2026-07-13T12:00:00Z", "2026-07-13T13:00:00Z");
    const missing = make("a", null, null);
    expect(
      [missing, present].sort((a, b) => compareDashboardUpstreamAccounts(a, b, "createdAt")),
    ).toEqual([present, missing]);
  });
});
