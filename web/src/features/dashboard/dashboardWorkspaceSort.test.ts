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

  it("sorts conversations by descending times and descending metrics", () => {
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
    const lower = make("b", 10, 20, 1, 10);
    const higher = make("a", 30, 40, 2, 20);
    expect(
      [lower, higher].sort((a, b) => compareDashboardConversationCards(a, b, "createdAt")),
    ).toEqual([higher, lower]);
    expect([lower, higher].sort((a, b) => compareDashboardConversationCards(a, b, "cost"))).toEqual(
      [higher, lower],
    );
    expect(
      [lower, higher].sort((a, b) => compareDashboardConversationCards(a, b, "tokens")),
    ).toEqual([higher, lower]);
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

  it("sorts upstream accounts by descending metrics and keeps unassigned rows last", () => {
    const make = (
      accountKey: string,
      upstreamAccountId: number | null,
      totalCost: number,
      totalTokens: number,
      latestConversationCreatedAt: string,
    ) =>
      ({
        accountKey,
        upstreamAccountId,
        isUnassigned: upstreamAccountId == null,
        displayName: accountKey,
        latestConversationCreatedAt,
        lastInvocationAt: latestConversationCreatedAt,
        totalCost,
        totalTokens,
      }) as UpstreamAccountActivityAccount;
    const assignedHigh = make("assigned-high", 42, 8, 800, "2026-07-13T13:00:00Z");
    const assignedLow = make("assigned-low", 43, 2, 200, "2026-07-13T12:00:00Z");
    const unassignedHighest = make("unassigned", null, 99, 9_999, "2026-07-13T14:00:00Z");

    expect(
      [assignedLow, unassignedHighest, assignedHigh].sort((a, b) =>
        compareDashboardUpstreamAccounts(a, b, "createdAt"),
      ),
    ).toEqual([assignedHigh, assignedLow, unassignedHighest]);
    expect(
      [assignedLow, unassignedHighest, assignedHigh].sort((a, b) =>
        compareDashboardUpstreamAccounts(a, b, "cost"),
      ),
    ).toEqual([assignedHigh, assignedLow, unassignedHighest]);
    expect(
      [assignedLow, unassignedHighest, assignedHigh].sort((a, b) =>
        compareDashboardUpstreamAccounts(a, b, "tokens"),
      ),
    ).toEqual([assignedHigh, assignedLow, unassignedHighest]);
  });
});
