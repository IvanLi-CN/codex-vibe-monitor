import type {
  InvocationModelRerouteFilter,
  InvocationModelTarget,
  InvocationRangePreset,
  InvocationRecordsQuery,
  InvocationSortBy,
  InvocationSortOrder,
  InvocationSuggestionField,
} from "./api";

export const RECORDS_PAGE_SIZE_OPTIONS = [20, 50, 100] as const;
export const RECORDS_NEW_COUNT_POLL_INTERVAL_MS = 15_000;
export const DEFAULT_RECORDS_FOCUS = "token" as const;
export const DEFAULT_RECORDS_SORT_BY: InvocationSortBy = "occurredAt";
export const DEFAULT_RECORDS_SORT_ORDER: InvocationSortOrder = "desc";
export const DEFAULT_RECORDS_PAGE_SIZE = RECORDS_PAGE_SIZE_OPTIONS[0];

export interface InvocationRecordsDraftFilters {
  rangePreset: InvocationRangePreset;
  customFrom: string;
  customTo: string;
  status: string;
  model: string;
  models: string[];
  modelTarget: InvocationModelTarget;
  modelRerouted: InvocationModelRerouteFilter;
  endpoint: string;
  invokeId: string;
  attemptId: string;
  failureClass: string;
  failureKind: string;
  promptCacheKey: string;
  upstreamScope: string;
  upstreamAccount: string;
  upstreamAccountId: string;
  proxyDisplayName: string;
  transport: string;
  serviceTier: string;
  reasoningEffort: string;
  reasoningEfforts: string[];
  requesterIp: string;
  keyword: string;
  minTotalTokens: string;
  maxTotalTokens: string;
  minTotalMs: string;
  maxTotalMs: string;
}

export interface InvocationRecordsDraftValidation {
  timeRange: "invalid" | "order" | null;
  totalTokens: "invalid" | "integer" | "order" | null;
  totalMs: "invalid" | "order" | null;
  modelFilters: "missingModel" | null;
}

export function createDefaultInvocationRecordsDraft(): InvocationRecordsDraftFilters {
  return {
    rangePreset: "today",
    customFrom: "",
    customTo: "",
    status: "",
    model: "",
    models: [],
    modelTarget: "request",
    modelRerouted: "all",
    endpoint: "",
    invokeId: "",
    attemptId: "",
    failureClass: "",
    failureKind: "",
    promptCacheKey: "",
    upstreamScope: "",
    upstreamAccount: "",
    upstreamAccountId: "",
    proxyDisplayName: "",
    transport: "",
    serviceTier: "",
    reasoningEffort: "",
    reasoningEfforts: [],
    requesterIp: "",
    keyword: "",
    minTotalTokens: "",
    maxTotalTokens: "",
    minTotalMs: "",
    maxTotalMs: "",
  };
}

function toIsoString(date: Date) {
  return date.toISOString();
}

function isMinutePrecisionLocalDateTimeValue(value: string) {
  // `datetime-local` defaults to "YYYY-MM-DDTHH:mm" (minute precision).
  // When we send that as an exclusive `< to` bound, it unintentionally excludes the whole minute.
  return /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(value);
}

function resolveCustomToUpperBound(value: string) {
  const parsed = new Date(value);
  if (isMinutePrecisionLocalDateTimeValue(value)) {
    return new Date(parsed.getTime() + 60_000);
  }
  return parsed;
}

function toLocalDateTimeValue(date: Date) {
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

export function createDefaultCustomRange(now = new Date()) {
  const from = new Date(now);
  from.setHours(0, 0, 0, 0);
  return {
    customFrom: toLocalDateTimeValue(from),
    customTo: toLocalDateTimeValue(now),
  };
}

function normalizeText(value: string) {
  const normalized = value.trim();
  return normalized ? normalized : undefined;
}

function normalizeTextList(values: string[]) {
  const deduped = new Set<string>();
  for (const value of values) {
    const normalized = value.trim();
    if (!normalized) continue;
    deduped.add(normalized);
  }
  return Array.from(deduped);
}

function resolveModelList(draft: InvocationRecordsDraftFilters) {
  const models = normalizeTextList(draft.models);
  const legacyModel = normalizeText(draft.model);
  if (models.length === 0 && legacyModel) {
    return [legacyModel];
  }
  return models;
}

function resolveReasoningEffortList(draft: InvocationRecordsDraftFilters) {
  const reasoningEfforts = normalizeTextList(draft.reasoningEfforts);
  const legacyReasoningEffort = normalizeText(draft.reasoningEffort);
  if (reasoningEfforts.length === 0 && legacyReasoningEffort) {
    return [legacyReasoningEffort];
  }
  return reasoningEfforts;
}

function resolveModelReroutedQueryValue(draft: InvocationRecordsDraftFilters) {
  if (draft.modelRerouted === "rerouted") return true;
  if (draft.modelRerouted === "notRerouted") return false;
  return undefined;
}

function parseDateInput(value: string) {
  const normalized = value.trim();
  if (!normalized) {
    return {
      date: undefined,
      invalid: false,
    };
  }
  const parsed = new Date(normalized);
  if (Number.isNaN(parsed.getTime())) {
    return {
      date: undefined,
      invalid: true,
    };
  }
  return {
    date: parsed,
    invalid: false,
  };
}

function normalizeNumber(value: string) {
  const normalized = value.trim();
  if (!normalized) return undefined;
  const parsed = Number(normalized);
  return Number.isFinite(parsed) ? parsed : undefined;
}

function normalizeInteger(value: string, fieldName: string) {
  const normalized = value.trim();
  if (!normalized) return undefined;
  const parsed = Number(normalized);
  if (!Number.isFinite(parsed)) return undefined;
  if (!Number.isInteger(parsed)) {
    throw new Error(`${fieldName} must be a whole number`);
  }
  return parsed;
}

function normalizeIntegerSafely(value: string, fieldName: string) {
  try {
    return normalizeInteger(value, fieldName);
  } catch {
    return undefined;
  }
}

function normalizeUpstreamAccountId(draft: InvocationRecordsDraftFilters) {
  const explicitId = normalizeIntegerSafely(draft.upstreamAccountId, "upstreamAccountId");
  if (explicitId !== undefined) {
    return explicitId;
  }
  return normalizeIntegerSafely(draft.upstreamAccount, "upstreamAccount");
}

export function validateInvocationRecordsDraft(
  draft: InvocationRecordsDraftFilters,
): InvocationRecordsDraftValidation {
  const timeRange: InvocationRecordsDraftValidation["timeRange"] = (() => {
    if (draft.rangePreset !== "custom") return null;
    const parsedFrom = parseDateInput(draft.customFrom);
    const parsedTo = parseDateInput(draft.customTo);
    if (parsedFrom.invalid || parsedTo.invalid) {
      return "invalid";
    }
    if (parsedFrom.date && parsedTo.date) {
      const exclusiveTo = resolveCustomToUpperBound(draft.customTo);
      if (parsedFrom.date.getTime() >= exclusiveTo.getTime()) {
        return "order";
      }
    }
    return null;
  })();

  const totalTokens: InvocationRecordsDraftValidation["totalTokens"] = (() => {
    const minValue = draft.minTotalTokens.trim();
    const maxValue = draft.maxTotalTokens.trim();
    const min = normalizeIntegerSafely(minValue, "minTotalTokens");
    const max = normalizeIntegerSafely(maxValue, "maxTotalTokens");
    if ((minValue && min === undefined) || (maxValue && max === undefined)) {
      const invalidValue = [minValue, maxValue]
        .filter((value) => value.length > 0)
        .find((value) => Number.isFinite(Number(value)) && !Number.isInteger(Number(value)));
      return invalidValue ? "integer" : "invalid";
    }
    if (min !== undefined && max !== undefined && min > max) {
      return "order";
    }
    return null;
  })();

  const totalMs: InvocationRecordsDraftValidation["totalMs"] = (() => {
    const minValue = draft.minTotalMs.trim();
    const maxValue = draft.maxTotalMs.trim();
    const min = normalizeNumber(minValue);
    const max = normalizeNumber(maxValue);
    if ((minValue && min === undefined) || (maxValue && max === undefined)) {
      return "invalid";
    }
    if (min !== undefined && max !== undefined && min > max) {
      return "order";
    }
    return null;
  })();

  const modelFilters: InvocationRecordsDraftValidation["modelFilters"] =
    resolveReasoningEffortList(draft).length > 0 && resolveModelList(draft).length === 0
      ? "missingModel"
      : null;

  return {
    timeRange,
    totalTokens,
    totalMs,
    modelFilters,
  };
}

function resolveRangeBoundsSafely(
  rangePreset: InvocationRangePreset,
  draft: InvocationRecordsDraftFilters,
  now = new Date(),
) {
  try {
    return resolveRangeBounds(rangePreset, draft, now);
  } catch {
    return { from: undefined, to: undefined };
  }
}

export function resolveRangeBoundsFromValues(
  rangePreset: InvocationRangePreset,
  customFrom: string,
  customTo: string,
  now = new Date(),
) {
  if (rangePreset === "custom") {
    const parsedFrom = parseDateInput(customFrom);
    const parsedTo = parseDateInput(customTo);
    if (parsedFrom.invalid || parsedTo.invalid) {
      throw new Error("Invalid time range");
    }
    if (parsedFrom.date && parsedTo.date) {
      const exclusiveTo = resolveCustomToUpperBound(customTo);
      if (parsedFrom.date.getTime() >= exclusiveTo.getTime()) {
        throw new Error("Time range must end after it starts");
      }
    }
    return {
      from: parsedFrom.date ? toIsoString(parsedFrom.date) : undefined,
      // Treat minute-based inputs as inclusive-of-minute for UX, while keeping server-side `< to` bounds.
      to: parsedTo.date ? toIsoString(resolveCustomToUpperBound(customTo)) : undefined,
    };
  }

  const end = new Date(now);
  const start = new Date(now);
  switch (rangePreset) {
    case "today":
      start.setHours(0, 0, 0, 0);
      end.setDate(end.getDate() + 1);
      end.setHours(0, 0, 0, 0);
      break;
    case "1d":
      start.setDate(start.getDate() - 1);
      break;
    case "7d":
      start.setDate(start.getDate() - 7);
      break;
    case "30d":
      start.setDate(start.getDate() - 30);
      break;
    default:
      break;
  }

  return {
    from: toIsoString(start),
    to: toIsoString(end),
  };
}

export function resolveRangeBounds(
  rangePreset: InvocationRangePreset,
  draft: InvocationRecordsDraftFilters,
  now = new Date(),
) {
  return resolveRangeBoundsFromValues(rangePreset, draft.customFrom, draft.customTo, now);
}

export function buildAppliedInvocationFilters(
  draft: InvocationRecordsDraftFilters,
  now = new Date(),
): Omit<InvocationRecordsQuery, "page" | "pageSize" | "sortBy" | "sortOrder" | "snapshotId"> {
  const validation = validateInvocationRecordsDraft(draft);
  if (validation.timeRange === "invalid") {
    throw new Error("Invalid time range");
  }
  if (validation.timeRange === "order") {
    throw new Error("Time range must end after it starts");
  }
  if (validation.totalTokens === "invalid") {
    throw new Error("Total tokens range must use numbers");
  }
  if (validation.totalTokens === "integer") {
    throw new Error("Total tokens range must use whole numbers");
  }
  if (validation.totalTokens === "order") {
    throw new Error("Total tokens range must be in ascending order");
  }
  if (validation.totalMs === "invalid") {
    throw new Error("Total ms range must use numbers");
  }
  if (validation.totalMs === "order") {
    throw new Error("Total ms range must be in ascending order");
  }
  if (validation.modelFilters === "missingModel") {
    throw new Error("Model filter requires at least one model");
  }
  const bounds = resolveRangeBounds(draft.rangePreset, draft, now);
  const models = resolveModelList(draft);
  const reasoningEfforts = resolveReasoningEffortList(draft);
  return {
    rangePreset: draft.rangePreset,
    from: bounds.from,
    to: bounds.to,
    status: normalizeText(draft.status),
    model: models.length === 1 ? models[0] : undefined,
    models: models.length > 0 ? models : undefined,
    modelTarget: models.length > 0 ? draft.modelTarget : undefined,
    modelRerouted: resolveModelReroutedQueryValue(draft),
    endpoint: normalizeText(draft.endpoint),
    invokeId: normalizeText(draft.invokeId),
    attemptId: normalizeText(draft.attemptId),
    failureClass: normalizeText(draft.failureClass),
    failureKind: normalizeText(draft.failureKind),
    promptCacheKey: normalizeText(draft.promptCacheKey),
    upstreamScope: normalizeText(draft.upstreamScope),
    upstreamAccountId: normalizeUpstreamAccountId(draft),
    proxyDisplayName: normalizeText(draft.proxyDisplayName),
    transport: normalizeText(draft.transport),
    serviceTier: normalizeText(draft.serviceTier),
    reasoningEffort: reasoningEfforts.length === 1 ? reasoningEfforts[0] : undefined,
    reasoningEfforts: reasoningEfforts.length > 0 ? reasoningEfforts : undefined,
    requesterIp: normalizeText(draft.requesterIp),
    keyword: normalizeText(draft.keyword),
    minTotalTokens: normalizeInteger(draft.minTotalTokens, "minTotalTokens"),
    maxTotalTokens: normalizeInteger(draft.maxTotalTokens, "maxTotalTokens"),
    minTotalMs: normalizeNumber(draft.minTotalMs),
    maxTotalMs: normalizeNumber(draft.maxTotalMs),
  };
}

function readSuggestionDraftValue(
  draft: InvocationRecordsDraftFilters,
  field?: InvocationSuggestionField,
) {
  switch (field) {
    case "model":
      return draft.model;
    case "requestModel":
    case "responseModel":
      return undefined;
    case "endpoint":
      return draft.endpoint;
    case "failureKind":
      return draft.failureKind;
    case "stickyKey":
      return undefined;
    case "promptCacheKey":
      return draft.promptCacheKey;
    case "requesterIp":
      return draft.requesterIp;
    case "proxyDisplayName":
      return draft.proxyDisplayName;
    case "upstreamAccount":
      return draft.upstreamAccount;
    case "serviceTier":
      return draft.serviceTier;
    case "reasoningEffort":
      return draft.reasoningEffort;
    default:
      return undefined;
  }
}

export function buildInvocationSuggestionsQuery(
  draft: InvocationRecordsDraftFilters,
  snapshotId?: number,
  suggestField?: InvocationSuggestionField,
  now = new Date(),
  suggestQueryOverride?: string,
): Omit<InvocationRecordsQuery, "page" | "pageSize" | "sortBy" | "sortOrder"> {
  const bounds = resolveRangeBoundsSafely(draft.rangePreset, draft, now);
  const models = resolveModelList(draft);
  const reasoningEfforts = resolveReasoningEffortList(draft);
  return {
    rangePreset: draft.rangePreset,
    from: bounds.from,
    to: bounds.to,
    status: normalizeText(draft.status),
    model: models.length === 1 ? models[0] : undefined,
    models: models.length > 0 ? models : undefined,
    modelTarget: models.length > 0 ? draft.modelTarget : undefined,
    modelRerouted: resolveModelReroutedQueryValue(draft),
    endpoint: normalizeText(draft.endpoint),
    invokeId: normalizeText(draft.invokeId),
    attemptId: normalizeText(draft.attemptId),
    failureClass: normalizeText(draft.failureClass),
    failureKind: normalizeText(draft.failureKind),
    promptCacheKey: normalizeText(draft.promptCacheKey),
    upstreamScope: normalizeText(draft.upstreamScope),
    upstreamAccountId: normalizeUpstreamAccountId(draft),
    proxyDisplayName: normalizeText(draft.proxyDisplayName),
    transport: normalizeText(draft.transport),
    serviceTier: normalizeText(draft.serviceTier),
    reasoningEffort: reasoningEfforts.length === 1 ? reasoningEfforts[0] : undefined,
    reasoningEfforts: reasoningEfforts.length > 0 ? reasoningEfforts : undefined,
    requesterIp: normalizeText(draft.requesterIp),
    keyword: normalizeText(draft.keyword),
    minTotalTokens: normalizeIntegerSafely(draft.minTotalTokens, "minTotalTokens"),
    maxTotalTokens: normalizeIntegerSafely(draft.maxTotalTokens, "maxTotalTokens"),
    minTotalMs: normalizeNumber(draft.minTotalMs),
    maxTotalMs: normalizeNumber(draft.maxTotalMs),
    suggestField,
    suggestQuery: suggestField
      ? normalizeText(suggestQueryOverride ?? readSuggestionDraftValue(draft, suggestField) ?? "")
      : undefined,
    snapshotId,
  };
}

export function buildInvocationRecordsQuery(
  base: Omit<InvocationRecordsQuery, "page" | "pageSize" | "sortBy" | "sortOrder" | "snapshotId">,
  options: {
    page: number;
    pageSize: number;
    sortBy: InvocationSortBy;
    sortOrder: InvocationSortOrder;
    snapshotId?: number;
  },
): InvocationRecordsQuery {
  return {
    ...base,
    page: options.page,
    pageSize: options.pageSize,
    sortBy: options.sortBy,
    sortOrder: options.sortOrder,
    snapshotId: options.snapshotId,
  };
}
