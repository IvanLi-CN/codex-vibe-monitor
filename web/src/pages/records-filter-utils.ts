import type { InvocationRangePreset } from "../lib/api";
import type { InvocationRecordsDraftFilters } from "../lib/invocationRecords";

export type ClearableRecordFilterKey = Exclude<
  keyof InvocationRecordsDraftFilters,
  "rangePreset" | "customFrom" | "customTo"
>;

export interface ActiveFilterChip {
  id: string;
  label: string;
  clearKeys?: ClearableRecordFilterKey[];
}

type RecordsTranslation = (key: string) => string;

export function formatCustomRange(from: string, to: string) {
  const values = [from, to].filter(Boolean).map((value) => value.replace("T", " "));
  return values.join(" - ");
}

function formatNumericRange(min: string, max: string) {
  const normalizedMin = min.trim();
  const normalizedMax = max.trim();
  if (normalizedMin && normalizedMax) return `${normalizedMin} - ${normalizedMax}`;
  if (normalizedMin) return `>= ${normalizedMin}`;
  if (normalizedMax) return `<= ${normalizedMax}`;
  return "";
}

function formatListSummary(values: string[]) {
  const normalized = values.map((value) => value.trim()).filter(Boolean);
  if (normalized.length === 0) return "";
  if (normalized.length <= 2) return normalized.join(", ");
  return `${normalized.slice(0, 2).join(", ")} +${normalized.length - 2}`;
}

function buildFilterLabelMaps(t: RecordsTranslation) {
  return {
    status: {
      success: t("records.filters.status.success"),
      warning_success: t("records.filters.status.warningSuccess"),
      failed: t("records.filters.status.failed"),
      interrupted: t("records.filters.status.interrupted"),
      running: t("records.filters.status.running"),
      pending: t("records.filters.status.pending"),
    },
    failureClass: {
      service_failure: t("records.filters.failureClass.service"),
      client_failure: t("records.filters.failureClass.client"),
      client_abort: t("records.filters.failureClass.abort"),
    },
    upstreamScope: {
      internal: t("records.filters.upstreamScope.internal"),
      external: t("records.filters.upstreamScope.external"),
    },
    transport: {
      http: t("records.filters.transport.http"),
      websocket: t("records.filters.transport.websocket"),
    },
    modelTarget: {
      request: t("records.filters.modelTarget.request"),
      response: t("records.filters.modelTarget.response"),
    },
    modelRerouted: {
      rerouted: t("records.filters.modelRerouted.rerouted"),
      notRerouted: t("records.filters.modelRerouted.notRerouted"),
    },
  } as {
    status: Record<string, string>;
    failureClass: Record<string, string>;
    upstreamScope: Record<string, string>;
    transport: Record<string, string>;
    modelTarget: Record<string, string>;
    modelRerouted: Record<string, string>;
  };
}

function appendTextFilterChip(
  chips: ActiveFilterChip[],
  draftKey: ClearableRecordFilterKey,
  label: string,
  value: string,
) {
  const normalized = value.trim();
  if (!normalized) return;
  chips.push({ id: draftKey, clearKeys: [draftKey], label: `${label}: ${normalized}` });
}

function appendRangeFilterChip(
  chips: ActiveFilterChip[],
  id: string,
  label: string,
  value: string,
  clearKeys: ClearableRecordFilterKey[],
) {
  const normalized = value.trim();
  if (!normalized) return;
  chips.push({ id, clearKeys, label: `${label}: ${normalized}` });
}

function buildModelFilterSummary(
  draft: InvocationRecordsDraftFilters,
  labels: ReturnType<typeof buildFilterLabelMaps>,
  t: RecordsTranslation,
) {
  const models =
    draft.models.length > 0 ? draft.models : draft.model.trim() ? [draft.model.trim()] : [];
  const reasoningEfforts =
    draft.reasoningEfforts.length > 0
      ? draft.reasoningEfforts
      : draft.reasoningEffort.trim()
        ? [draft.reasoningEffort.trim()]
        : [];
  return [
    models.length > 0 ? labels.modelTarget[draft.modelTarget] : null,
    models.length > 0 ? formatListSummary(models) : null,
    reasoningEfforts.length > 0
      ? `${t("records.filters.reasoningEffort")}: ${formatListSummary(reasoningEfforts)}`
      : null,
    draft.modelRerouted !== "all" ? labels.modelRerouted[draft.modelRerouted] : null,
  ].filter((value): value is string => Boolean(value));
}

function appendCommonFilterChips(
  chips: ActiveFilterChip[],
  draft: InvocationRecordsDraftFilters,
  labels: ReturnType<typeof buildFilterLabelMaps>,
  t: RecordsTranslation,
) {
  appendTextFilterChip(
    chips,
    "status",
    t("records.filters.status"),
    labels.status[draft.status] ?? draft.status,
  );
  const modelSummary = buildModelFilterSummary(draft, labels, t);
  if (modelSummary.length > 0) {
    chips.push({
      id: "modelSelection",
      clearKeys: [
        "model",
        "models",
        "modelTarget",
        "modelRerouted",
        "reasoningEffort",
        "reasoningEfforts",
      ],
      label: `${t("records.filters.model")}: ${modelSummary.join(" · ")}`,
    });
  }
  appendTextFilterChip(chips, "endpoint", t("records.filters.endpoint"), draft.endpoint);
  appendTextFilterChip(
    chips,
    "failureClass",
    t("records.filters.failureClass"),
    labels.failureClass[draft.failureClass] ?? draft.failureClass,
  );
  appendTextFilterChip(chips, "invokeId", t("records.filters.invokeId"), draft.invokeId);
  appendTextFilterChip(chips, "attemptId", t("records.filters.attemptId"), draft.attemptId);
  appendTextFilterChip(chips, "failureKind", t("records.filters.failureKind"), draft.failureKind);
  appendTextFilterChip(
    chips,
    "promptCacheKey",
    t("records.filters.promptCacheKey"),
    draft.promptCacheKey,
  );
  appendTextFilterChip(
    chips,
    "upstreamScope",
    t("records.filters.upstreamScope"),
    labels.upstreamScope[draft.upstreamScope] ?? draft.upstreamScope,
  );
}

export function buildActiveFilterChips(
  appliedDraft: InvocationRecordsDraftFilters | null,
  rangeOptions: Array<{ value: InvocationRangePreset; label: string }>,
  t: RecordsTranslation,
): ActiveFilterChip[] {
  if (!appliedDraft) return [];
  const rangeLabel =
    appliedDraft.rangePreset === "custom"
      ? formatCustomRange(appliedDraft.customFrom, appliedDraft.customTo) ||
        t("records.filters.rangePreset.custom")
      : (rangeOptions.find((option) => option.value === appliedDraft.rangePreset)?.label ??
        t("records.filters.rangePreset"));
  const chips: ActiveFilterChip[] = [
    { id: "range", label: `${t("records.filters.rangePreset")}: ${rangeLabel}` },
  ];
  const labels = buildFilterLabelMaps(t);
  appendCommonFilterChips(chips, appliedDraft, labels, t);
  if (appliedDraft.upstreamAccount.trim()) {
    chips.push({
      id: "upstreamAccount",
      clearKeys: ["upstreamAccount", "upstreamAccountId"],
      label: `${t("records.filters.upstreamAccount")}: ${appliedDraft.upstreamAccount.trim()}`,
    });
  }
  appendTextFilterChip(
    chips,
    "transport",
    t("records.filters.transport"),
    labels.transport[appliedDraft.transport] ?? appliedDraft.transport,
  );
  appendTextFilterChip(
    chips,
    "proxyDisplayName",
    t("records.filters.proxyDisplayName"),
    appliedDraft.proxyDisplayName,
  );
  appendTextFilterChip(
    chips,
    "serviceTier",
    t("records.filters.serviceTier"),
    appliedDraft.serviceTier,
  );
  appendTextFilterChip(
    chips,
    "requesterIp",
    t("records.filters.requesterIp"),
    appliedDraft.requesterIp,
  );
  appendTextFilterChip(chips, "keyword", t("records.filters.keyword"), appliedDraft.keyword);
  appendRangeFilterChip(
    chips,
    "totalTokensRange",
    t("records.filters.totalTokensRange"),
    formatNumericRange(appliedDraft.minTotalTokens, appliedDraft.maxTotalTokens),
    ["minTotalTokens", "maxTotalTokens"],
  );
  appendRangeFilterChip(
    chips,
    "totalMsRange",
    t("records.filters.totalMsRange"),
    formatNumericRange(appliedDraft.minTotalMs, appliedDraft.maxTotalMs),
    ["minTotalMs", "maxTotalMs"],
  );
  return chips;
}
