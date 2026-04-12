import type {
  GroupFilterState,
  PersistedUpstreamAccountsFilters,
} from "./UpstreamAccounts.shared-types";
import {
  DEFAULT_GROUP_FILTER_STATE,
  DEFAULT_PERSISTED_UPSTREAM_ACCOUNT_FILTERS,
} from "./UpstreamAccounts.shared-types";

const UPSTREAM_ACCOUNTS_FILTER_STORAGE_KEY =
  "codex-vibe-monitor.account-pool.upstream-accounts.filters";

const WORK_STATUS_FILTER_VALUES = [
  "working",
  "degraded",
  "idle",
  "rate_limited",
  "unavailable",
] as const;

const ENABLE_STATUS_FILTER_VALUES = ["enabled", "disabled"] as const;

const HEALTH_STATUS_FILTER_VALUES = [
  "normal",
  "needs_reauth",
  "upstream_unavailable",
  "upstream_rejected",
  "error_other",
] as const;

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function sanitizeFilterValues(
  value: unknown,
  allowedValues: readonly string[],
): string[] {
  if (!Array.isArray(value)) return [];
  const allowed = new Set(allowedValues);
  const next: string[] = [];
  for (const item of value) {
    if (typeof item !== "string" || !allowed.has(item) || next.includes(item)) {
      continue;
    }
    next.push(item);
  }
  return next;
}

function sanitizeTagIds(value: unknown): number[] {
  if (!Array.isArray(value)) return [];
  const next: number[] = [];
  for (const item of value) {
    if (!Number.isInteger(item) || item <= 0 || next.includes(item)) {
      continue;
    }
    next.push(item);
  }
  return next;
}

function sanitizeGroupFilterState(value: unknown): GroupFilterState {
  if (!isPlainObject(value)) return DEFAULT_GROUP_FILTER_STATE;
  const mode = value.mode;
  if (mode === "ungrouped") {
    return {
      mode,
      query: "",
    };
  }
  if (mode === "search") {
    const query = typeof value.query === "string" ? value.query.trim() : "";
    if (query) {
      return {
        mode,
        query,
      };
    }
  }
  return DEFAULT_GROUP_FILTER_STATE;
}

export function readPersistedUpstreamAccountFilters(): PersistedUpstreamAccountsFilters {
  if (typeof window === "undefined") {
    return DEFAULT_PERSISTED_UPSTREAM_ACCOUNT_FILTERS;
  }
  try {
    const raw = window.localStorage.getItem(
      UPSTREAM_ACCOUNTS_FILTER_STORAGE_KEY,
    );
    if (!raw) {
      return DEFAULT_PERSISTED_UPSTREAM_ACCOUNT_FILTERS;
    }
    const parsed = JSON.parse(raw);
    if (!isPlainObject(parsed)) {
      return DEFAULT_PERSISTED_UPSTREAM_ACCOUNT_FILTERS;
    }
    return {
      workStatus: sanitizeFilterValues(
        parsed.workStatus,
        WORK_STATUS_FILTER_VALUES,
      ),
      enableStatus: sanitizeFilterValues(
        parsed.enableStatus,
        ENABLE_STATUS_FILTER_VALUES,
      ),
      healthStatus: sanitizeFilterValues(
        parsed.healthStatus,
        HEALTH_STATUS_FILTER_VALUES,
      ),
      tagIds: sanitizeTagIds(parsed.tagIds),
      groupFilter: sanitizeGroupFilterState(parsed.groupFilter),
    };
  } catch {
    return DEFAULT_PERSISTED_UPSTREAM_ACCOUNT_FILTERS;
  }
}

export function persistUpstreamAccountFilters(
  value: PersistedUpstreamAccountsFilters,
): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(
      UPSTREAM_ACCOUNTS_FILTER_STORAGE_KEY,
      JSON.stringify(value),
    );
  } catch {
    // Ignore storage write failures and keep the current UI state.
  }
}

export function formatGroupFilterValue(
  groupFilter: GroupFilterState,
  labels: { ungrouped: string },
): string {
  if (groupFilter.mode === "ungrouped") {
    return labels.ungrouped;
  }
  if (groupFilter.mode === "search") {
    return groupFilter.query;
  }
  return "";
}

export function parseGroupFilterValue(
  value: string,
  labels: { all: string; ungrouped: string },
): GroupFilterState {
  const normalized = value.trim();
  if (!normalized) {
    return DEFAULT_GROUP_FILTER_STATE;
  }
  const normalizedLower = normalized.toLocaleLowerCase();
  if (normalizedLower === labels.all.trim().toLocaleLowerCase()) {
    return DEFAULT_GROUP_FILTER_STATE;
  }
  if (normalizedLower === labels.ungrouped.trim().toLocaleLowerCase()) {
    return {
      mode: "ungrouped",
      query: "",
    };
  }
  return {
    mode: "search",
    query: normalized,
  };
}
