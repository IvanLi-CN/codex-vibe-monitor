import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import type { ModelPerformanceModel, UsageBreakdownModel } from "../../lib/api";

export type DashboardModelBreakdownMode = "simple" | "detailed";
export type DashboardModelBreakdownWindow = "model-performance" | "usage-breakdown";
export type DashboardModelSortRule = "name-asc" | "name-desc" | "effort-asc" | "effort-desc";
export type DashboardSortDirection = "asc" | "desc";
export type DashboardModelPerformanceSortColumn =
  | "tpm"
  | "streaming-rate"
  | "response"
  | "first-byte"
  | "wall-clock-duration"
  | "cumulative-duration"
  | "parallelism";
export type DashboardUsageBreakdownSortColumn = "cache-hit-rate" | "total";
export type DashboardModelBreakdownSort =
  | { column: "model"; rule: DashboardModelSortRule }
  | {
      column: DashboardModelPerformanceSortColumn | DashboardUsageBreakdownSortColumn;
      direction: DashboardSortDirection;
    };

export const DASHBOARD_MODEL_BREAKDOWN_MODE_STORAGE_KEY =
  "codex-vibe-monitor.dashboard.model-breakdown.mode.v1";
export const DASHBOARD_MODEL_BREAKDOWN_SORT_STORAGE_KEY_PREFIX =
  "codex-vibe-monitor.dashboard.model-breakdown.sort.v1";

export const REASONING_EFFORT_SORT_ORDER = [
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
  "ultra",
] as const;

const DEFAULT_SORTS: Record<DashboardModelBreakdownWindow, DashboardModelBreakdownSort> = {
  "model-performance": { column: "cumulative-duration", direction: "desc" },
  "usage-breakdown": { column: "total", direction: "desc" },
};

const PERFORMANCE_SORT_COLUMNS = new Set<DashboardModelPerformanceSortColumn>([
  "tpm",
  "streaming-rate",
  "response",
  "first-byte",
  "wall-clock-duration",
  "cumulative-duration",
  "parallelism",
]);
const USAGE_SORT_COLUMNS = new Set<DashboardUsageBreakdownSortColumn>(["cache-hit-rate", "total"]);

const listeners = new Set<() => void>();
const PREFERENCE_EVENT = "codex-vibe-monitor.dashboard.model-breakdown.preference";

function notifyPreferenceListeners() {
  for (const listener of listeners) listener();
}

function subscribeToPreferences(listener: () => void) {
  listeners.add(listener);
  if (typeof window === "undefined") return () => listeners.delete(listener);

  const handleStorage = (event: StorageEvent) => {
    if (
      event.key === DASHBOARD_MODEL_BREAKDOWN_MODE_STORAGE_KEY ||
      event.key?.startsWith(`${DASHBOARD_MODEL_BREAKDOWN_SORT_STORAGE_KEY_PREFIX}.`)
    ) {
      listener();
    }
  };
  window.addEventListener("storage", handleStorage);
  window.addEventListener(PREFERENCE_EVENT, listener);
  return () => {
    listeners.delete(listener);
    window.removeEventListener("storage", handleStorage);
    window.removeEventListener(PREFERENCE_EVENT, listener);
  };
}

function readStorage(key: string) {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeStorage(key: string, value: string) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Ignore storage failures and keep the floating window interactive.
  }
}

function emitPreferenceEvent() {
  if (typeof window === "undefined") {
    notifyPreferenceListeners();
    return;
  }
  window.dispatchEvent(new Event(PREFERENCE_EVENT));
}

function isMode(value: string | null): value is DashboardModelBreakdownMode {
  return value === "simple" || value === "detailed";
}

export function readDashboardModelBreakdownMode(): DashboardModelBreakdownMode {
  const value = readStorage(DASHBOARD_MODEL_BREAKDOWN_MODE_STORAGE_KEY);
  return isMode(value) ? value : "detailed";
}

export function persistDashboardModelBreakdownMode(mode: DashboardModelBreakdownMode) {
  writeStorage(DASHBOARD_MODEL_BREAKDOWN_MODE_STORAGE_KEY, mode);
  emitPreferenceEvent();
}

export function useDashboardModelBreakdownMode() {
  const mode = useSyncExternalStore(
    subscribeToPreferences,
    readDashboardModelBreakdownMode,
    () => "detailed" as DashboardModelBreakdownMode,
  );
  const setMode = useCallback((nextMode: DashboardModelBreakdownMode) => {
    persistDashboardModelBreakdownMode(nextMode);
  }, []);
  return [mode, setMode] as const;
}

function sortStorageKey(windowKey: DashboardModelBreakdownWindow) {
  return `${DASHBOARD_MODEL_BREAKDOWN_SORT_STORAGE_KEY_PREFIX}.${windowKey}`;
}

function isModelSortRule(value: unknown): value is DashboardModelSortRule {
  return (
    value === "name-asc" ||
    value === "name-desc" ||
    value === "effort-asc" ||
    value === "effort-desc"
  );
}

function isDirection(value: unknown): value is DashboardSortDirection {
  return value === "asc" || value === "desc";
}

function isSortColumn(windowKey: DashboardModelBreakdownWindow, value: unknown): boolean {
  if (value === "model") return true;
  return windowKey === "model-performance"
    ? PERFORMANCE_SORT_COLUMNS.has(value as DashboardModelPerformanceSortColumn)
    : USAGE_SORT_COLUMNS.has(value as DashboardUsageBreakdownSortColumn);
}

function isPerformanceSortColumn(
  value: DashboardModelBreakdownSort["column"],
): value is DashboardModelPerformanceSortColumn {
  return (
    value !== "model" && PERFORMANCE_SORT_COLUMNS.has(value as DashboardModelPerformanceSortColumn)
  );
}

function isUsageSortColumn(
  value: DashboardModelBreakdownSort["column"],
): value is DashboardUsageBreakdownSortColumn {
  return value !== "model" && USAGE_SORT_COLUMNS.has(value as DashboardUsageBreakdownSortColumn);
}

function parseSort(
  windowKey: DashboardModelBreakdownWindow,
  value: string | null,
): DashboardModelBreakdownSort {
  if (value) {
    try {
      const candidate = JSON.parse(value) as Record<string, unknown>;
      if (candidate.column === "model" && isModelSortRule(candidate.rule)) {
        return { column: "model", rule: candidate.rule };
      }
      if (isSortColumn(windowKey, candidate.column) && isDirection(candidate.direction)) {
        return {
          column: candidate.column as
            | DashboardModelPerformanceSortColumn
            | DashboardUsageBreakdownSortColumn,
          direction: candidate.direction,
        };
      }
    } catch {
      // Fall through to the stable default for malformed persisted state.
    }
  }
  return DEFAULT_SORTS[windowKey];
}

export function readDashboardModelBreakdownSort(
  windowKey: DashboardModelBreakdownWindow,
): DashboardModelBreakdownSort {
  return parseSort(windowKey, readStorage(sortStorageKey(windowKey)));
}

export function persistDashboardModelBreakdownSort(
  windowKey: DashboardModelBreakdownWindow,
  sort: DashboardModelBreakdownSort,
) {
  writeStorage(sortStorageKey(windowKey), JSON.stringify(sort));
  emitPreferenceEvent();
}

export function useDashboardModelBreakdownSort(windowKey: DashboardModelBreakdownWindow) {
  const [sort, setSort] = useState(() => readDashboardModelBreakdownSort(windowKey));

  useEffect(() => {
    setSort(readDashboardModelBreakdownSort(windowKey));
    return subscribeToPreferences(() => {
      setSort(readDashboardModelBreakdownSort(windowKey));
    });
  }, [windowKey]);

  const updateSort = useCallback(
    (nextSort: DashboardModelBreakdownSort) => {
      persistDashboardModelBreakdownSort(windowKey, nextSort);
      setSort(nextSort);
    },
    [windowKey],
  );

  return [sort, updateSort] as const;
}

export function modelSortRuleDirection(rule: DashboardModelSortRule): DashboardSortDirection {
  return rule.endsWith("asc") ? "asc" : "desc";
}

export function isEffortSortRule(rule: DashboardModelSortRule) {
  return rule.startsWith("effort-");
}

const MODEL_SORT_RULES: readonly DashboardModelSortRule[] = [
  "name-asc",
  "name-desc",
  "effort-asc",
  "effort-desc",
];

export function nextModelSortRule(
  current: DashboardModelSortRule | null | undefined,
  simple: boolean,
): DashboardModelSortRule {
  const availableRules = simple ? MODEL_SORT_RULES.slice(0, 2) : MODEL_SORT_RULES;
  const currentIndex = current == null ? -1 : availableRules.indexOf(current);
  return availableRules[(currentIndex + 1) % availableRules.length] ?? "name-asc";
}

export function sortForMode(
  sort: DashboardModelBreakdownSort,
  mode: DashboardModelBreakdownMode,
): DashboardModelBreakdownSort {
  if (mode === "simple" && sort.column === "model" && isEffortSortRule(sort.rule)) {
    return { column: "model", rule: "name-asc" };
  }
  return sort;
}

function modelNameCompare(left: string, right: string, localeTag: string) {
  return left.localeCompare(right, localeTag, { sensitivity: "base" });
}

function effortSortRank(value: string | null | undefined) {
  const normalized = value?.trim().toLowerCase();
  const index = REASONING_EFFORT_SORT_ORDER.indexOf(
    normalized as (typeof REASONING_EFFORT_SORT_ORDER)[number],
  );
  return index === -1 ? null : index;
}

function compareEfforts(
  left: string | null | undefined,
  right: string | null | undefined,
  direction: DashboardSortDirection,
) {
  const leftRank = effortSortRank(left);
  const rightRank = effortSortRank(right);
  if (leftRank == null || rightRank == null) {
    if (leftRank == null && rightRank == null) return 0;
    return leftRank == null ? 1 : -1;
  }
  const difference = leftRank - rightRank;
  return direction === "asc" ? difference : -difference;
}

function compareModelIdentity(
  left: { model: string; reasoningEffort?: string | null },
  right: { model: string; reasoningEffort?: string | null },
  rule: DashboardModelSortRule,
  localeTag: string,
) {
  const direction = modelSortRuleDirection(rule);
  const primary = rule.startsWith("name-")
    ? modelNameCompare(left.model, right.model, localeTag) * (direction === "asc" ? 1 : -1)
    : compareEfforts(left.reasoningEffort, right.reasoningEffort, direction);
  return (
    primary ||
    modelNameCompare(left.model, right.model, localeTag) ||
    compareEfforts(left.reasoningEffort, right.reasoningEffort, "asc")
  );
}

function compareNumeric(
  left: number | null | undefined,
  right: number | null | undefined,
  direction: DashboardSortDirection,
) {
  const leftPresent = left != null && Number.isFinite(left);
  const rightPresent = right != null && Number.isFinite(right);
  if (!leftPresent || !rightPresent) {
    if (!leftPresent && !rightPresent) return 0;
    return leftPresent ? -1 : 1;
  }
  const difference = (left as number) - (right as number);
  return direction === "asc" ? difference : -difference;
}

function performanceMetricValue(
  model: ModelPerformanceModel,
  column: DashboardModelPerformanceSortColumn,
) {
  switch (column) {
    case "tpm":
      return model.tokensPerMinute;
    case "streaming-rate":
      return model.streamingResponseRate;
    case "response":
      return model.avgResponseMs;
    case "first-byte":
      return model.avgFirstTokenMs;
    case "wall-clock-duration":
      return model.wallClockUsageDurationMs;
    case "cumulative-duration":
      return model.cumulativeUsageDurationMs;
    case "parallelism":
      return model.parallelism;
  }
}

export function sortModelPerformanceModels(
  models: readonly ModelPerformanceModel[],
  sort: DashboardModelBreakdownSort,
  localeTag: string,
) {
  return [...models].sort((left, right) => {
    const primary =
      sort.column === "model"
        ? compareModelIdentity(left, right, sort.rule, localeTag)
        : isPerformanceSortColumn(sort.column)
          ? compareNumeric(
              performanceMetricValue(left, sort.column),
              performanceMetricValue(right, sort.column),
              sort.direction,
            )
          : 0;
    return primary || compareModelIdentity(left, right, "name-asc", localeTag);
  });
}

function usageBreakdownModelHasData(model: UsageBreakdownModel) {
  return (
    model.costs != null ||
    model.cacheWriteTokens > 0 ||
    model.cacheReadTokens > 0 ||
    model.outputTokens > 0
  );
}

function mergeCosts(
  left: UsageBreakdownModel["costs"],
  right: UsageBreakdownModel["costs"],
): NonNullable<UsageBreakdownModel["costs"]> | null {
  if (!left && !right) return null;
  return {
    input: (left?.input ?? 0) + (right?.input ?? 0),
    cacheWrite: (left?.cacheWrite ?? 0) + (right?.cacheWrite ?? 0),
    cacheRead: (left?.cacheRead ?? 0) + (right?.cacheRead ?? 0),
    output: (left?.output ?? 0) + (right?.output ?? 0),
    reasoning: (left?.reasoning ?? 0) + (right?.reasoning ?? 0),
    unknown: (left?.unknown ?? 0) + (right?.unknown ?? 0),
  };
}

export function groupUsageBreakdownModels(models: readonly UsageBreakdownModel[]) {
  const grouped = new Map<string, UsageBreakdownModel>();
  for (const model of models) {
    const modelName = model.model.trim() || "unknown";
    const current = grouped.get(modelName);
    if (!current) {
      grouped.set(modelName, {
        ...model,
        model: modelName,
        reasoningEffort: null,
        costs: model.costs ? { ...model.costs } : null,
      });
      continue;
    }
    current.cacheWriteTokens += model.cacheWriteTokens;
    current.cacheReadTokens += model.cacheReadTokens;
    current.outputTokens += model.outputTokens;
    current.costs = mergeCosts(current.costs, model.costs);
  }
  return [...grouped.values()].filter(usageBreakdownModelHasData);
}

function usageModelMetricValue(
  model: UsageBreakdownModel,
  column: DashboardUsageBreakdownSortColumn,
) {
  switch (column) {
    case "cache-hit-rate": {
      const total = model.cacheWriteTokens + model.cacheReadTokens + model.outputTokens;
      return total > 0 ? Math.max(model.cacheReadTokens, 0) / total : null;
    }
    case "total":
      return model.cacheWriteTokens + model.cacheReadTokens + model.outputTokens;
  }
}

export function sortUsageBreakdownModels(
  models: readonly UsageBreakdownModel[],
  sort: DashboardModelBreakdownSort,
  localeTag: string,
) {
  return [...models].sort((left, right) => {
    const primary =
      sort.column === "model"
        ? compareModelIdentity(left, right, sort.rule, localeTag)
        : isUsageSortColumn(sort.column)
          ? compareNumeric(
              usageModelMetricValue(left, sort.column),
              usageModelMetricValue(right, sort.column),
              sort.direction,
            )
          : 0;
    return primary || compareModelIdentity(left, right, "name-asc", localeTag);
  });
}
