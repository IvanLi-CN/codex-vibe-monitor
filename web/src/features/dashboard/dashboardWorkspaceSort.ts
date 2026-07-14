import type { UpstreamAccountActivityAccount } from "../../lib/api";
import type { DashboardWorkingConversationCardModel } from "../../lib/dashboardWorkingConversations";

export type DashboardWorkspaceSort = "createdAt" | "lastInvocation" | "cost" | "tokens";

export const DASHBOARD_CONVERSATION_SORT_STORAGE_KEY =
  "codex-vibe-monitor.dashboard.workspace.conversations.sort";
export const DASHBOARD_UPSTREAM_ACCOUNT_SORT_STORAGE_KEY =
  "codex-vibe-monitor.dashboard.workspace.upstream-accounts.sort";

const SORT_CYCLE: DashboardWorkspaceSort[] = ["createdAt", "lastInvocation", "cost", "tokens"];

export function nextDashboardWorkspaceSort(value: DashboardWorkspaceSort): DashboardWorkspaceSort {
  return SORT_CYCLE[(SORT_CYCLE.indexOf(value) + 1) % SORT_CYCLE.length] ?? "createdAt";
}

export function readDashboardWorkspaceSort(storageKey: string): DashboardWorkspaceSort {
  if (typeof window === "undefined") return "createdAt";
  const value = window.localStorage.getItem(storageKey);
  return SORT_CYCLE.includes(value as DashboardWorkspaceSort)
    ? (value as DashboardWorkspaceSort)
    : "createdAt";
}

export function persistDashboardWorkspaceSort(storageKey: string, value: DashboardWorkspaceSort) {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(storageKey, value);
}

function compareOptionalEpochDesc(left: number | null, right: number | null) {
  if (left == null) return right == null ? 0 : 1;
  if (right == null) return -1;
  return right - left;
}

function compareNumberDesc(left: number, right: number) {
  return right - left;
}

function parseEpoch(value: string | null | undefined) {
  if (!value) return null;
  const epoch = Date.parse(value);
  return Number.isNaN(epoch) ? null : epoch;
}

export function compareDashboardConversationCards(
  left: DashboardWorkingConversationCardModel,
  right: DashboardWorkingConversationCardModel,
  sort: DashboardWorkspaceSort,
) {
  const primary =
    sort === "createdAt"
      ? compareOptionalEpochDesc(left.createdAtEpoch, right.createdAtEpoch)
      : sort === "lastInvocation"
        ? compareOptionalEpochDesc(
            left.currentInvocation.occurredAtEpoch,
            right.currentInvocation.occurredAtEpoch,
          )
        : sort === "cost"
          ? compareNumberDesc(left.totalCost, right.totalCost)
          : compareNumberDesc(left.totalTokens, right.totalTokens);
  return primary || left.promptCacheKey.localeCompare(right.promptCacheKey);
}

export function compareDashboardUpstreamAccounts(
  left: UpstreamAccountActivityAccount,
  right: UpstreamAccountActivityAccount,
  sort: DashboardWorkspaceSort,
) {
  const leftUnassigned = left.isUnassigned === true || left.upstreamAccountId == null;
  const rightUnassigned = right.isUnassigned === true || right.upstreamAccountId == null;
  if (leftUnassigned !== rightUnassigned) {
    return leftUnassigned ? 1 : -1;
  }
  const primary =
    sort === "createdAt"
      ? compareOptionalEpochDesc(
          parseEpoch(left.latestConversationCreatedAt),
          parseEpoch(right.latestConversationCreatedAt),
        )
      : sort === "lastInvocation"
        ? compareOptionalEpochDesc(
            parseEpoch(left.lastInvocationAt),
            parseEpoch(right.lastInvocationAt),
          )
        : sort === "cost"
          ? compareNumberDesc(left.totalCost, right.totalCost)
          : compareNumberDesc(left.totalTokens, right.totalTokens);
  const leftKey = left.accountKey ?? String(left.upstreamAccountId ?? "unassigned");
  const rightKey = right.accountKey ?? String(right.upstreamAccountId ?? "unassigned");
  return primary || leftKey.localeCompare(rightKey);
}
