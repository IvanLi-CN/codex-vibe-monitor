import type {
  ForwardProxyBindingNode,
  UpstreamAccountGroupSummary,
  UpstreamAccountSummary,
} from "./api";

export type AccountPoolGroupPlanCount = {
  key: string;
  label: string;
  count: number;
};

export interface AccountPoolGroupSummaryData {
  id: string;
  groupName: string | null;
  displayName: string;
  items: UpstreamAccountSummary[];
  note?: string | null;
  boundProxyKeys?: string[];
  boundProxyLabels?: string[];
  concurrencyLimit?: number | null;
  nodeShuntEnabled?: boolean;
  upstream429RetryEnabled?: boolean;
  upstream429MaxRetries?: number;
  hasCustomSettings?: boolean;
  planCounts: AccountPoolGroupPlanCount[];
}

export function normalizeAccountPoolGroupName(value?: string | null) {
  const normalized = value?.trim();
  return normalized ? normalized : null;
}

const GROUPED_PLAN_ORDER = ["free", "pro", "team", "enterprise"];

export function buildAccountPoolGroupSummaries(options: {
  items: UpstreamAccountSummary[];
  groups: UpstreamAccountGroupSummary[];
  forwardProxyNodes: ForwardProxyBindingNode[];
  ungroupedLabel: string;
  groupedPlanLabel: (planType?: string | null) => string | null;
}): AccountPoolGroupSummaryData[] {
  const {
    items,
    groups,
    forwardProxyNodes,
    ungroupedLabel,
    groupedPlanLabel,
  } = options;

  const normalizedGroupEntries = groups.map((group, index) => ({
    group,
    index,
    normalizedGroupName: normalizeAccountPoolGroupName(group.groupName),
  }));
  const namedGroupEntries = normalizedGroupEntries.filter(
    (
      entry,
    ): entry is {
      group: UpstreamAccountGroupSummary;
      index: number;
      normalizedGroupName: string;
    } => entry.normalizedGroupName != null,
  );
  const forwardProxyNodeLabelMap = new Map(
    forwardProxyNodes.map((node) => [
      node.key,
      node.displayName?.trim() || node.key,
    ] as const),
  );
  const groupSummaryMap = new Map(
    namedGroupEntries.map((entry) => [entry.normalizedGroupName, entry.group] as const),
  );
  const groupOrder = new Map(
    namedGroupEntries.map((entry) => [entry.normalizedGroupName, entry.index] as const),
  );
  const grouped = new Map<string, AccountPoolGroupSummaryData>();

  for (const item of items) {
    const normalizedGroupName = normalizeAccountPoolGroupName(item.groupName);
    const groupKey = normalizedGroupName ?? "__ungrouped__";
    const groupSummary = normalizedGroupName
      ? groupSummaryMap.get(normalizedGroupName) ?? null
      : null;
    const current = grouped.get(groupKey) ?? {
      id: groupKey,
      groupName: normalizedGroupName,
      displayName: normalizedGroupName ?? ungroupedLabel,
      items: [],
      note: groupSummary?.note ?? null,
      boundProxyKeys: groupSummary?.boundProxyKeys ?? [],
      boundProxyLabels:
        groupSummary?.boundProxyKeys?.map(
          (proxyKey) => forwardProxyNodeLabelMap.get(proxyKey) ?? proxyKey,
        ) ?? [],
      concurrencyLimit: groupSummary?.concurrencyLimit ?? null,
      nodeShuntEnabled: groupSummary?.nodeShuntEnabled ?? false,
      upstream429RetryEnabled: groupSummary?.upstream429RetryEnabled ?? false,
      upstream429MaxRetries: groupSummary?.upstream429MaxRetries ?? 0,
      hasCustomSettings:
        Boolean(groupSummary?.note?.trim()) ||
        (groupSummary?.boundProxyKeys?.length ?? 0) > 0 ||
        (groupSummary?.concurrencyLimit ?? 0) > 0 ||
        groupSummary?.nodeShuntEnabled === true ||
        groupSummary?.upstream429RetryEnabled === true ||
        (groupSummary?.upstream429MaxRetries ?? 0) > 0,
      planCounts: [],
    };
    current.items.push(item);
    grouped.set(groupKey, current);
  }

  const result = Array.from(grouped.values()).map((group) => {
    const counts = new Map<string, number>();
    for (const item of group.items) {
      if (item.kind === "api_key_codex") {
        counts.set("api", (counts.get("api") ?? 0) + 1);
      }
      const normalizedPlan = item.planType?.trim().toLowerCase();
      if (!normalizedPlan || normalizedPlan === "local") continue;
      counts.set(normalizedPlan, (counts.get(normalizedPlan) ?? 0) + 1);
    }

    const orderedKeys = [
      ...GROUPED_PLAN_ORDER.filter((key) => counts.has(key)),
      ...(counts.has("api") ? ["api"] : []),
      ...Array.from(counts.keys())
        .filter((key) => key !== "api" && !GROUPED_PLAN_ORDER.includes(key))
        .sort(),
    ];

    return {
      ...group,
      planCounts: orderedKeys
        .map((key) => ({
          key,
          label: key === "api" ? "API" : groupedPlanLabel(key) ?? key,
          count: counts.get(key) ?? 0,
        }))
        .filter((plan) => plan.count > 0),
    };
  });

  result.sort((left, right) => {
    const leftOrder =
      left.groupName == null
        ? Number.MAX_SAFE_INTEGER
        : (groupOrder.get(left.groupName) ?? Number.MAX_SAFE_INTEGER - 1);
    const rightOrder =
      right.groupName == null
        ? Number.MAX_SAFE_INTEGER
        : (groupOrder.get(right.groupName) ?? Number.MAX_SAFE_INTEGER - 1);
    return (
      leftOrder - rightOrder || left.displayName.localeCompare(right.displayName)
    );
  });

  return result;
}
