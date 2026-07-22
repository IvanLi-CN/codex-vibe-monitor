import type { UpstreamAccountSummary } from "../../lib/api";

export type DashboardBulkRouteBindRecentTarget =
  | {
      kind: "group";
      groupName: string;
      usedAt: number;
    }
  | {
      kind: "upstreamAccount";
      upstreamAccountId: number;
      usedAt: number;
    };

export const DASHBOARD_BULK_ROUTE_BIND_RECENT_TARGETS_STORAGE_KEY =
  "codex-vibe-monitor.dashboard.bulk-route-bind.recent-targets.v1";

export const DASHBOARD_BULK_ROUTE_BIND_RECENT_TARGETS_MAX = 5;

function resolveDashboardBulkRouteBindStorage(storage?: Storage): Storage | undefined {
  return storage ?? (typeof window === "undefined" ? undefined : window.localStorage);
}

function normalizeGroupName(value?: string | null): string {
  return value?.trim() ?? "";
}

function toFiniteTimestamp(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function recentTargetStableKey(target: DashboardBulkRouteBindRecentTarget): string {
  return target.kind === "group"
    ? `group:${target.groupName}`
    : `upstreamAccount:${target.upstreamAccountId}`;
}

function sanitizeDashboardBulkRouteBindRecentTarget(
  value: unknown,
): DashboardBulkRouteBindRecentTarget | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  const candidate = value as Record<string, unknown>;
  const usedAt = toFiniteTimestamp(candidate.usedAt);
  if (usedAt == null) {
    return null;
  }
  if (candidate.kind === "group") {
    const groupName = normalizeGroupName(
      typeof candidate.groupName === "string" ? candidate.groupName : null,
    );
    if (!groupName) {
      return null;
    }
    return {
      kind: "group",
      groupName,
      usedAt,
    };
  }
  if (candidate.kind === "upstreamAccount") {
    const upstreamAccountId = candidate.upstreamAccountId;
    if (
      typeof upstreamAccountId !== "number" ||
      !Number.isInteger(upstreamAccountId) ||
      upstreamAccountId <= 0
    ) {
      return null;
    }
    return {
      kind: "upstreamAccount",
      upstreamAccountId,
      usedAt,
    };
  }
  return null;
}

export function normalizeDashboardBulkRouteBindRecentTargets(
  targets: DashboardBulkRouteBindRecentTarget[],
): DashboardBulkRouteBindRecentTarget[] {
  const deduped = new Map<string, DashboardBulkRouteBindRecentTarget>();
  for (const target of targets) {
    const key = recentTargetStableKey(target);
    const current = deduped.get(key);
    if (!current || target.usedAt > current.usedAt) {
      deduped.set(key, target);
    }
  }
  return Array.from(deduped.values())
    .sort((left, right) => {
      if (left.usedAt !== right.usedAt) {
        return right.usedAt - left.usedAt;
      }
      return recentTargetStableKey(left).localeCompare(recentTargetStableKey(right));
    })
    .slice(0, DASHBOARD_BULK_ROUTE_BIND_RECENT_TARGETS_MAX);
}

export function readDashboardBulkRouteBindRecentTargets(
  storage?: Storage,
): DashboardBulkRouteBindRecentTarget[] {
  try {
    const resolvedStorage = resolveDashboardBulkRouteBindStorage(storage);
    if (!resolvedStorage) {
      return [];
    }
    const raw = resolvedStorage.getItem(DASHBOARD_BULK_ROUTE_BIND_RECENT_TARGETS_STORAGE_KEY);
    if (!raw) {
      return [];
    }
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }
    return normalizeDashboardBulkRouteBindRecentTargets(
      parsed
        .map((target) => sanitizeDashboardBulkRouteBindRecentTarget(target))
        .filter((target): target is DashboardBulkRouteBindRecentTarget => target != null),
    );
  } catch {
    return [];
  }
}

export function persistDashboardBulkRouteBindRecentTargets(
  targets: DashboardBulkRouteBindRecentTarget[],
  storage?: Storage,
): void {
  try {
    const resolvedStorage = resolveDashboardBulkRouteBindStorage(storage);
    if (!resolvedStorage) {
      return;
    }
    const normalized = normalizeDashboardBulkRouteBindRecentTargets(targets);
    if (normalized.length === 0) {
      resolvedStorage.removeItem(DASHBOARD_BULK_ROUTE_BIND_RECENT_TARGETS_STORAGE_KEY);
      return;
    }
    resolvedStorage.setItem(
      DASHBOARD_BULK_ROUTE_BIND_RECENT_TARGETS_STORAGE_KEY,
      JSON.stringify(normalized),
    );
  } catch {
    // Ignore storage failures; the route-bind memory is only a local preference.
  }
}

export function rememberDashboardBulkRouteBindRecentTarget(
  currentTargets: DashboardBulkRouteBindRecentTarget[],
  target:
    | {
        kind: "group";
        groupName: string;
      }
    | {
        kind: "upstreamAccount";
        upstreamAccountId: number;
      },
  usedAt = Date.now(),
): DashboardBulkRouteBindRecentTarget[] {
  return normalizeDashboardBulkRouteBindRecentTargets([
    ...currentTargets,
    {
      ...target,
      usedAt,
    },
  ]);
}

export function filterAvailableDashboardBulkRouteBindRecentTargets(
  targets: DashboardBulkRouteBindRecentTarget[],
  options: {
    groups: string[];
    accounts: ReadonlyArray<Pick<UpstreamAccountSummary, "id">>;
  },
): DashboardBulkRouteBindRecentTarget[] {
  const groupSet = new Set(
    options.groups.map((groupName) => normalizeGroupName(groupName)).filter(Boolean),
  );
  const accountIdSet = new Set(
    options.accounts
      .map((account) => account.id)
      .filter((accountId) => Number.isInteger(accountId) && accountId > 0),
  );
  return normalizeDashboardBulkRouteBindRecentTargets(
    targets.filter((target) =>
      target.kind === "group"
        ? groupSet.has(target.groupName)
        : accountIdSet.has(target.upstreamAccountId),
    ),
  );
}

export function isDashboardBulkRouteBindRecentTargetSelected(
  target: DashboardBulkRouteBindRecentTarget,
  selection: {
    kind: "group" | "upstreamAccount";
    groupName: string;
    upstreamAccountId: string;
  },
): boolean {
  return target.kind === "group"
    ? selection.kind === "group" && target.groupName === selection.groupName
    : selection.kind === "upstreamAccount" &&
        String(target.upstreamAccountId) === selection.upstreamAccountId;
}
