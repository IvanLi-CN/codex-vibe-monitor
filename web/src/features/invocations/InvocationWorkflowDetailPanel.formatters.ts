import type { ChipTone } from "../../components/ui/chip";
import type { TranslationKey } from "../../i18n";
import type {
  ApiInvocation,
  InvocationCostAudit,
  InvocationCostAuditBreakdown,
} from "../../lib/api";
import {
  formatDashboardWorkingConversationSequenceId,
  hashDashboardWorkingConversationKey,
} from "../../lib/dashboardWorkingConversations";
import { resolveInvocationEndpointDisplay } from "../../lib/invocation";
import {
  isFiniteNonNegativeMilliseconds,
  isFinitePositiveMilliseconds,
} from "../../lib/invocationTiming";
import { formatReasoningEffort } from "../shared/reasoningEffort";

export type DetailPanelSize = "compact" | "default";
export type AttemptSection =
  | "timing"
  | "requestParsed"
  | "requestHeaders"
  | "requestBody"
  | "responseParsed"
  | "responseHeaders"
  | "responseBody";
export type GenericSection = "request" | "requestHeaders" | "requestBody" | "json" | "body";
export type Translator = (key: TranslationKey, values?: Record<string, string | number>) => string;

export interface PayloadFetchState<T> {
  status: "idle" | "loading" | "loaded" | "error";
  data: T | null;
  error: string | null;
}

export interface AttemptUsageAudit {
  inputTokens: number | null;
  cacheWriteTokens: number | null;
  cacheInputTokens: number | null;
  outputTokens: number | null;
  reasoningTokens: number | null;
  totalTokens: number | null;
  recordedCosts: InvocationCostAuditBreakdown | null;
  localCosts: InvocationCostAuditBreakdown | null;
  audit: InvocationCostAudit | null;
}

export interface TimelineFact {
  key: string;
  label: string;
  tooltip?: string;
  tone?: ChipTone;
}

export interface InvocationWorkflowDetailPanelProps {
  record: ApiInvocation;
  focusedAttemptId?: string | null;
  size?: DetailPanelSize;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  hideNonShortIds?: boolean;
}

export const FALLBACK_CELL = "—";

export function formatDurationMs(value: number | null | undefined, locale: string) {
  if (!isFiniteNonNegativeMilliseconds(value)) return FALLBACK_CELL;
  const seconds = value / 1000;
  const rounded = Math.round(seconds * 10) / 10;
  const precision = Math.abs(rounded) >= 100 ? 0 : 1;
  return `${seconds.toLocaleString(locale, {
    minimumFractionDigits: 0,
    maximumFractionDigits: precision,
  })} s`;
}

export function formatResponseDurationMs(value: number | null | undefined, locale: string) {
  if (!isFinitePositiveMilliseconds(value)) return FALLBACK_CELL;
  return formatDurationMs(value, locale);
}

export function formatMilliseconds(value: number | null | undefined, locale: string) {
  if (!isFiniteNonNegativeMilliseconds(value)) return FALLBACK_CELL;
  return `${value.toLocaleString(locale, {
    minimumFractionDigits: 0,
    maximumFractionDigits: 1,
  })} ms`;
}

export function formatResponseMilliseconds(value: number | null | undefined, locale: string) {
  if (!isFinitePositiveMilliseconds(value)) return FALLBACK_CELL;
  return formatMilliseconds(value, locale);
}

export function formatTimestamp(value: string | null | undefined, locale: string) {
  const normalized = value?.trim();
  if (!normalized) return FALLBACK_CELL;
  const parsed = new Date(normalized);
  if (Number.isNaN(parsed.getTime())) return normalized;
  return new Intl.DateTimeFormat(locale, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(parsed);
}

export function formatOptionalText(value: string | null | undefined) {
  const normalized = value?.trim();
  return normalized ? normalized : FALLBACK_CELL;
}

export function formatNoCandidateReason(code: string, isZh: boolean) {
  const labels: Record<string, [string, string]> = {
    modelConcurrencyLimit: ["模型并发容量已满", "Model concurrency capacity is full"],
    expiredCooldownProbe: ["冷却后的探针容量已占用", "Expired cooldown probe capacity is occupied"],
    stickyRouteReservationConflict: ["粘性路由预约冲突", "Sticky-route reservation conflict"],
    policyExcluded: ["路由策略已排除", "Routing policy excluded the candidate"],
    noEligibleCandidate: ["没有符合条件的候选账号", "No eligible account candidate"],
    bindingConstraint: ["不符合当前会话绑定条件", "Excluded by the current binding constraint"],
    requiredRouteMismatch: ["不符合要求的路由", "Does not match the required route"],
    recentTransportFailure: ["最近发生过传输失败", "Recently had a transport failure"],
    previousAttemptExcluded: ["已被本请求的先前尝试排除", "Excluded by an earlier attempt"],
    stickyReuseUnavailable: ["无法继续复用粘性路由", "Sticky route cannot be reused"],
    rateLimited: ["账号已被限流", "Account is rate limited"],
    degraded: ["账号处于降级状态", "Account is degraded"],
    notSelectableForFreshAssignment: ["不允许接收重新分配", "Not selectable for reassignment"],
    unavailable: ["账号当前不可用", "Account is unavailable"],
    modelNotAllowed: ["不允许当前请求模型", "Requested model is not allowed"],
    capabilityUnsupported: ["不支持当前请求能力", "Requested capability is unsupported"],
    concurrencyLimit: ["账号并发容量已满", "Account concurrency capacity is full"],
    stickyPolicy: ["粘性策略已排除", "Excluded by sticky routing policy"],
    forwardProxyUnavailable: ["转发代理不可用", "Forward proxy is unavailable"],
    modelTemporarilyExcluded: ["模型路由暂时不可用", "Model route is temporarily unavailable"],
    notAssignable: ["账号不可分配", "Account is not assignable"],
  };
  return labels[code]?.[isZh ? 0 : 1] ?? (isZh ? "未知路由原因" : "Unknown routing reason");
}

export function formatReasoningEffortValue(value: unknown) {
  return formatReasoningEffort(typeof value === "string" ? value : null);
}

export function normalizeEndpointCompactionKind(
  value: string | null | undefined,
): ApiInvocation["compactionRequestKind"] {
  if (value === "compact" || value === "remote_v2") return value;
  return undefined;
}

export function resolveEndpointMetricDisplay({
  endpoint,
  status,
  compactionRequestKind,
  compactionResponseKind,
  t,
}: {
  endpoint: string | null | undefined;
  status: string | null | undefined;
  compactionRequestKind?: string | null | undefined;
  compactionResponseKind?: string | null | undefined;
  t: Translator;
}) {
  const display = resolveInvocationEndpointDisplay({
    endpoint: endpoint ?? undefined,
    status: status ?? undefined,
    compactionRequestKind: normalizeEndpointCompactionKind(compactionRequestKind),
    compactionResponseKind: normalizeEndpointCompactionKind(compactionResponseKind),
  });
  const value =
    display.labelKey && display.kind !== "raw"
      ? t(display.labelKey)
      : formatOptionalText(display.endpointValue);

  return {
    tag: display.kind === "raw" ? null : value,
    tertiary: display.kind === "raw" ? value : null,
  };
}

export function formatOptionalNumber(value: number | null | undefined, locale: string) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  return value.toLocaleString(locale);
}

export function formatCurrency(value: number | null | undefined, locale: string) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  return new Intl.NumberFormat(locale, {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 4,
    maximumFractionDigits: 4,
  }).format(value);
}

export function buildConversationShortId(promptCacheKey: string | null | undefined) {
  const normalized = promptCacheKey?.trim();
  if (!normalized) return FALLBACK_CELL;
  return formatDashboardWorkingConversationSequenceId(
    `WC-${hashDashboardWorkingConversationKey(normalized).slice(0, 6)}`,
  );
}

export function buildPayloadViewerLabels(isZh: boolean) {
  return {
    json: isZh ? "JSON 结构" : "JSON",
    ndjson: isZh ? "NDJSON 结构" : "NDJSON",
    sse: isZh ? "SSE 事件流" : "SSE",
    text: isZh ? "纯文本" : "Text",
    largePayload: isZh ? "内容较大，默认不自动结构化解析。" : "Large payload. Parsing is deferred.",
    parseLargePayload: isZh ? "解析结构" : "Parse",
    event: isZh ? "事件" : "Event",
    data: isZh ? "数据" : "Data",
    expand: isZh ? "展开" : "Expand",
    collapse: isZh ? "收起" : "Collapse",
  };
}

export function resolveStatusMeta(status: string | null | undefined, isZh: boolean) {
  const normalized = (status ?? "").trim().toLowerCase();
  if (normalized === "success" || normalized === "completed") {
    return { variant: "success" as const, label: isZh ? "成功" : "Success" };
  }
  if (normalized === "warning_success") {
    return {
      variant: "warning" as const,
      label: isZh ? "告警成功" : "Warning",
    };
  }
  if (normalized === "failed" || normalized === "transport_failure") {
    return { variant: "error" as const, label: isZh ? "失败" : "Failed" };
  }
  if (normalized === "http_failure") {
    return {
      variant: "error" as const,
      label: isZh ? "HTTP 失败" : "HTTP Failure",
    };
  }
  if (normalized === "budget_exhausted_final") {
    return {
      variant: "warning" as const,
      label: isZh ? "预算耗尽" : "Budget Exhausted",
    };
  }
  if (normalized === "running") {
    return { variant: "primary" as const, label: isZh ? "运行中" : "Running" };
  }
  if (normalized === "pending") {
    return {
      variant: "secondary" as const,
      label: isZh ? "等待中" : "Pending",
    };
  }
  if (normalized.startsWith("http_")) {
    return {
      variant: normalized.startsWith("http_4") ? ("warning" as const) : ("error" as const),
      label: normalized.toUpperCase().replace("_", " "),
    };
  }
  return {
    variant: "secondary" as const,
    label: status?.trim() || (isZh ? "未知" : "Unknown"),
  };
}

export function resolveKindMeta(kind: string, isZh: boolean) {
  switch (kind) {
    case "routingDecision":
      return {
        label: isZh ? "路由" : "Route",
        variant: "secondary" as const,
        markerClass: "border-info/55 bg-info/18 tone-ink-info",
      };
    case "routingWait":
      return {
        label: isZh ? "等待" : "Wait",
        variant: "secondary" as const,
        markerClass: "border-accent/55 bg-accent/18 tone-ink-accent",
      };
    case "systemFinalFailure":
      return {
        label: isZh ? "裁定" : "Final",
        variant: "warning" as const,
        markerClass: "border-warning/70 bg-warning/25 tone-ink-warning",
      };
    default:
      return {
        label: isZh ? "尝试" : "Attempt",
        variant: "primary" as const,
        markerClass: "border-primary/60 bg-primary/14 tone-ink-primary",
      };
  }
}

export function formatRouteMode(value: string | null | undefined, isZh: boolean) {
  const normalized = value?.trim().toLowerCase();
  if (!normalized) return FALLBACK_CELL;
  if (normalized === "pool") return isZh ? "号池" : "Pool";
  if (normalized === "forward_proxy") return isZh ? "代理直连" : "Forward Proxy";
  if (normalized === "direct") return isZh ? "直连" : "Direct";
  return value?.trim() ?? FALLBACK_CELL;
}

export function stringifyStructuredValue(value: Record<string, unknown> | null | undefined) {
  if (!value) return "";
  return JSON.stringify(value, null, 2);
}

export function readString(value: unknown) {
  if (typeof value !== "string") return null;
  const normalized = value.trim();
  return normalized ? normalized : null;
}

export function readNumber(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function readBoolean(value: unknown) {
  return typeof value === "boolean" ? value : null;
}

export function readRecord(value: unknown) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
}

export function readArray(value: unknown) {
  return Array.isArray(value) ? value : null;
}

export function readCostAuditBreakdown(value: unknown): InvocationCostAuditBreakdown | null {
  const record = readRecord(value);
  if (!record) return null;
  const input = readNumber(record.input);
  const cacheWrite = readNumber(record.cacheWrite);
  const cacheRead = readNumber(record.cacheRead);
  const output = readNumber(record.output);
  const reasoning = readNumber(record.reasoning);
  const total = readNumber(record.total);
  if (
    input == null &&
    cacheWrite == null &&
    cacheRead == null &&
    output == null &&
    reasoning == null &&
    total == null
  ) {
    return null;
  }
  return {
    input,
    cacheWrite,
    cacheRead,
    output,
    reasoning,
    total,
  };
}

export function readCostAudit(value: unknown): InvocationCostAudit | null {
  const record = readRecord(value);
  if (!record) return null;
  const recorded = readCostAuditBreakdown(record.recorded);
  const local = readCostAuditBreakdown(record.local);
  const mismatch = record.mismatch === true;
  const reason = readString(record.reason);
  const absoluteDiffUsd = readNumber(record.absoluteDiffUsd);
  const recordedPriceVersion = readString(record.recordedPriceVersion);
  const localPriceVersion = readString(record.localPriceVersion);
  if (
    recorded == null &&
    local == null &&
    !mismatch &&
    reason == null &&
    absoluteDiffUsd == null &&
    recordedPriceVersion == null &&
    localPriceVersion == null
  ) {
    return null;
  }
  return {
    recorded,
    local,
    mismatch,
    reason,
    absoluteDiffUsd,
    recordedPriceVersion,
    localPriceVersion,
  };
}

export function readAttemptUsageAudit(value: unknown): AttemptUsageAudit | null {
  const usage = readRecord(value);
  if (!usage) return null;
  const tokens = readRecord(usage.tokens);
  const costs = readRecord(usage.costs);
  const audit = readCostAudit(usage.audit);
  return {
    inputTokens: readNumber(usage.inputTokens) ?? readNumber(tokens?.input),
    cacheWriteTokens: readNumber(usage.cacheWriteTokens) ?? readNumber(tokens?.cacheWrite),
    cacheInputTokens: readNumber(usage.cacheInputTokens) ?? readNumber(tokens?.cacheRead),
    outputTokens: readNumber(usage.outputTokens) ?? readNumber(tokens?.output),
    reasoningTokens: readNumber(usage.reasoningTokens) ?? readNumber(tokens?.reasoning),
    totalTokens: readNumber(usage.totalTokens) ?? readNumber(tokens?.total),
    recordedCosts: readCostAuditBreakdown(costs?.recorded),
    localCosts: readCostAuditBreakdown(costs?.local),
    audit,
  };
}

export function formatBooleanLabel(value: boolean | null | undefined, isZh: boolean) {
  if (value == null) return FALLBACK_CELL;
  return value ? (isZh ? "是" : "Yes") : isZh ? "否" : "No";
}

export function formatUnknownValue(value: unknown, locale: string, isZh: boolean) {
  if (typeof value === "string") return formatOptionalText(value);
  if (typeof value === "number") return formatOptionalNumber(value, locale);
  if (typeof value === "boolean") return formatBooleanLabel(value, isZh);
  if (Array.isArray(value)) {
    const parts = value
      .map((entry) =>
        typeof entry === "string" ? entry.trim() : entry == null ? "" : JSON.stringify(entry),
      )
      .filter(Boolean);
    return parts.length > 0 ? parts.join(" · ") : FALLBACK_CELL;
  }
  if (value && typeof value === "object") {
    try {
      return JSON.stringify(value);
    } catch {
      return FALLBACK_CELL;
    }
  }
  return FALLBACK_CELL;
}

export function buildStructuredItems(
  source: Record<string, unknown> | null | undefined,
  locale: string,
  isZh: boolean,
  specs: Array<{
    key: string;
    label: string;
    monospace?: boolean;
    formatter?: (value: unknown) => string;
  }>,
) {
  if (!source) return [];
  return specs
    .map((spec) => {
      const rawValue = source[spec.key];
      const value = spec.formatter
        ? spec.formatter(rawValue)
        : formatUnknownValue(rawValue, locale, isZh);
      return {
        label: spec.label,
        value,
        monospace: spec.monospace,
      };
    })
    .filter((item) => item.value !== FALLBACK_CELL);
}

export function normalizeToolLabel(value: unknown) {
  const record = readRecord(value);
  if (!record) return null;
  const type = readString(record.type);
  const functionRecord = readRecord(record.function);
  const functionName = readString(functionRecord?.name);
  const name = readString(record.name);
  if (functionName && type) return `${type}:${functionName}`;
  if (functionName) return functionName;
  if (name && type) return `${type}:${name}`;
  if (name) return name;
  return type;
}

export function extractRequestBusinessSnapshot(bodyText: string) {
  const trimmed = bodyText.trim();
  if (!trimmed.startsWith("{") && !trimmed.startsWith("[")) return null;
  try {
    const parsed = JSON.parse(trimmed) as Record<string, unknown>;
    const tools = readArray(parsed.tools)
      ?.map(normalizeToolLabel)
      .filter((value): value is string => Boolean(value));
    const input = parsed.input;
    const messages = readArray(parsed.messages);
    const modalities = readArray(parsed.modalities)
      ?.map((entry) => (typeof entry === "string" ? entry.trim() : ""))
      .filter(Boolean);
    const textFormat = readRecord(readRecord(parsed.text)?.format);
    const responseFormat = readRecord(parsed.response_format);
    const toolChoice = readRecord(parsed.tool_choice);
    const reasoning = readRecord(parsed.reasoning);

    return {
      model: readString(parsed.model),
      stream: readBoolean(parsed.stream),
      serviceTier: readString(parsed.service_tier) ?? readString(parsed.serviceTier),
      reasoningEffort: readString(reasoning?.effort) ?? readString(parsed.reasoning_effort),
      maxOutputTokens: readNumber(parsed.max_output_tokens) ?? readNumber(parsed.maxOutputTokens),
      temperature: readNumber(parsed.temperature),
      topP: readNumber(parsed.top_p) ?? readNumber(parsed.topP),
      parallelToolCalls:
        readBoolean(parsed.parallel_tool_calls) ?? readBoolean(parsed.parallelToolCalls),
      toolChoice: readString(toolChoice?.type) ?? readString(parsed.tool_choice),
      tools,
      modalities,
      inputCount:
        readArray(input)?.length ??
        readArray(messages)?.length ??
        (typeof input === "string" ? 1 : null),
      inputShape:
        readArray(input) != null
          ? "array"
          : readArray(messages) != null
            ? "messages"
            : typeof input === "string"
              ? "text"
              : input && typeof input === "object"
                ? "object"
                : null,
      textFormat:
        readString(textFormat?.type) ??
        readString(responseFormat?.type) ??
        readString(parsed.response_format),
    };
  } catch {
    return null;
  }
}

export function extractResponseBusinessSnapshot(bodyText: string) {
  const trimmed = bodyText.trim();
  if (!trimmed.startsWith("{")) return null;
  try {
    const parsed = JSON.parse(trimmed) as Record<string, unknown>;
    const errorRecord = readRecord(parsed.error);
    const usage = readRecord(parsed.usage);
    const output = readArray(parsed.output);
    const outputTextBlocks =
      output?.flatMap((entry) => {
        const content = readArray(readRecord(entry)?.content);
        return (
          content
            ?.filter((item) => readString(readRecord(item)?.type) === "output_text")
            .map((item) => readString(readRecord(item)?.text) ?? "")
            .filter(Boolean) ?? []
        );
      }) ?? [];
    const toolCalls =
      output
        ?.map((entry) => {
          const record = readRecord(entry);
          if (!record) return null;
          const type = readString(record.type);
          const name = readString(readRecord(record.function)?.name) ?? readString(record.name);
          if (name && type) return `${type}:${name}`;
          return name ?? type;
        })
        .filter((value): value is string => Boolean(value)) ?? [];

    return {
      id: readString(parsed.id),
      object: readString(parsed.object) ?? readString(parsed.type),
      status: readString(parsed.status),
      model: readString(parsed.model),
      serviceTier: readString(parsed.service_tier) ?? readString(parsed.serviceTier),
      outputItems: output?.length ?? null,
      outputTextBlocks: outputTextBlocks.length > 0 ? outputTextBlocks.length : null,
      toolCalls,
      errorCode: readString(errorRecord?.code),
      errorMessage: readString(errorRecord?.message),
      usageInputTokens: readNumber(usage?.input_tokens) ?? readNumber(usage?.inputTokens),
      usageOutputTokens: readNumber(usage?.output_tokens) ?? readNumber(usage?.outputTokens),
      usageReasoningTokens:
        readNumber(usage?.reasoning_tokens) ?? readNumber(usage?.reasoningTokens),
      usageTotalTokens: readNumber(usage?.total_tokens) ?? readNumber(usage?.totalTokens),
    };
  } catch {
    return null;
  }
}

export function isRequestSection(section: AttemptSection) {
  return section === "requestParsed" || section === "requestHeaders" || section === "requestBody";
}

export function isResponseSection(section: AttemptSection) {
  return (
    section === "responseParsed" || section === "responseHeaders" || section === "responseBody"
  );
}

export interface TimelineMetricAction<TSection extends string> {
  section: TSection;
  label: string;
  tag?: string | null;
  primary: string;
  secondary?: string | null;
  secondaryTone?: "success";
  tertiary?: string | null;
  tertiaryChips?: string[] | null;
  tertiaryOverflowCount?: number;
  monospace?: boolean;
}

export function createIdlePayloadState<T>(): PayloadFetchState<T> {
  return {
    status: "idle",
    data: null,
    error: null,
  };
}

export function formatPayloadUnavailableReason(reason: string | null | undefined, isZh: boolean) {
  const normalized = reason?.trim().toLowerCase() ?? "";
  if (normalized === "not_abnormal") {
    return isZh
      ? "该记录没有异常响应体。"
      : "No abnormal response body is available for this record.";
  }
  if (normalized === "detail_pruned") {
    return isZh
      ? "该记录的完整载荷已不再在线保留。"
      : "The full payload for this record is no longer retained online.";
  }
  if (normalized.startsWith("raw_file_missing")) {
    return isZh ? "归档 raw 文件已不可用。" : "The archived raw file is no longer available.";
  }
  if (normalized.startsWith("raw_file_unreadable")) {
    return isZh ? "归档 raw 文件暂时无法读取。" : "The archived raw file could not be read.";
  }
  if (normalized.startsWith("preview_only")) {
    return isZh
      ? "该记录当前仅保留载荷节选。"
      : "Only a preview of this payload is currently available.";
  }
  if (normalized.startsWith("missing_body")) {
    return isZh
      ? "该记录没有保留可展示的载荷。"
      : "No displayable payload was retained for this record.";
  }
  if (normalized === "non_final_attempt_response_body_not_captured") {
    return isZh
      ? "该重试不是最终响应，未绑定调用级响应体。"
      : "This retry is not the final response, so invocation-level response body is not attached.";
  }
  if (normalized === "attempt_response_body_not_captured") {
    return isZh
      ? "该次上游尝试未保留可展示的响应体。"
      : "No displayable response body was retained for this upstream attempt.";
  }
  return isZh ? "载荷当前不可用。" : "The payload is currently unavailable.";
}

export function formatHttpStatus(value: number | null | undefined, locale: string) {
  const status = formatOptionalNumber(value, locale);
  if (status === FALLBACK_CELL) return null;
  return `HTTP ${status}`;
}

export function formatByteSize(value: number | null | undefined, locale: string) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  return `${value.toLocaleString(locale)} B`;
}

export function formatCompactByteSize(value: number | null | undefined, locale: string) {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) return FALLBACK_CELL;
  const units = ["B", "KB", "MB", "GB", "TB"];
  let scaled = value;
  let unitIndex = 0;
  while (scaled >= 1024 && unitIndex < units.length - 1) {
    scaled /= 1024;
    unitIndex += 1;
  }
  const maximumFractionDigits = unitIndex === 0 ? 0 : scaled >= 10 ? 1 : 1;
  return `${scaled.toLocaleString(locale, {
    minimumFractionDigits: unitIndex === 0 ? 0 : 1,
    maximumFractionDigits,
  })} ${units[unitIndex]}`;
}

export function formatSignedPercent(value: number | null | undefined, locale: string) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  const rounded = Math.round(value);
  if (rounded > 0) return `+${rounded.toLocaleString(locale)}%`;
  return `${rounded.toLocaleString(locale)}%`;
}

export function formatRequestCompressionSummary(
  compression: Record<string, unknown> | null | undefined,
  locale: string,
) {
  const logicalBodyBytes = readNumber(compression?.logicalBodyBytes);
  const transmittedBodyBytes = readNumber(compression?.transmittedBodyBytes);
  const ratioPct = readNumber(compression?.ratioPct);
  if (
    logicalBodyBytes == null ||
    transmittedBodyBytes == null ||
    ratioPct == null ||
    !Number.isFinite(logicalBodyBytes) ||
    !Number.isFinite(transmittedBodyBytes) ||
    !Number.isFinite(ratioPct)
  ) {
    return FALLBACK_CELL;
  }
  return `${formatSignedPercent(ratioPct, locale)} (${formatCompactByteSize(
    logicalBodyBytes,
    locale,
  )} -> ${formatCompactByteSize(transmittedBodyBytes, locale)})`;
}

export function compactJoin(parts: Array<string | null | undefined>) {
  const normalized = parts
    .map((part) => (typeof part === "string" ? part.trim() : ""))
    .filter((part) => part.length > 0 && part !== FALLBACK_CELL);
  return normalized.length > 0 ? normalized.join(" · ") : FALLBACK_CELL;
}

export function formatHttpCompressionTag(value: string | null | undefined) {
  const normalized = value?.trim().toLowerCase();
  if (!normalized) return null;
  return normalized;
}

export function formatCompactionSummary(value: string | null | undefined, isZh: boolean) {
  const normalized = value?.trim().toLowerCase();
  if (!normalized) return null;
  if (normalized === "remote_v2") return isZh ? "远程压缩V2" : "Remote compaction V2";
  if (normalized === "compact") return "Compact";
  return normalized;
}

export function summarizeToolCalls(value: unknown, isZh: boolean) {
  const tools = readArray(value)
    ?.map((entry) => (typeof entry === "string" ? entry.trim() : ""))
    .filter(Boolean);
  if (!tools || tools.length === 0) return null;
  if (tools.length === 1) return tools[0];
  return isZh ? `${tools.length} 个工具` : `${tools.length} tools`;
}

export function buildToolChips(value: unknown) {
  const tools =
    readArray(value)
      ?.map((entry) => {
        if (typeof entry !== "string") return null;
        const normalized = entry.trim();
        if (!normalized) return null;
        const segments = normalized
          .split(":")
          .map((segment) => segment.trim())
          .filter(Boolean);
        const label = segments.at(-1) ?? normalized;
        return label.endsWith("_preview") ? label.slice(0, -"_preview".length) : label;
      })
      .filter((entry): entry is string => Boolean(entry)) ?? [];
  if (tools.length === 0) return null;
  const uniqueTools = Array.from(new Set(tools));
  const maxVisible = 2;
  const characterBudget = 18;
  const visible: string[] = [];
  let used = 0;
  for (const tool of uniqueTools) {
    if (visible.length >= maxVisible) break;
    const nextUsed = used + tool.length;
    if (visible.length > 0 && nextUsed > characterBudget) break;
    visible.push(tool);
    used = nextUsed;
  }
  if (visible.length === 0) visible.push(uniqueTools[0]);
  return {
    visible,
    overflowCount: Math.max(0, uniqueTools.length - visible.length),
  };
}

export function summarizeOutputItems(value: unknown, locale: string, isZh: boolean) {
  const count = readNumber(value);
  if (count == null) return null;
  return isZh
    ? `${count.toLocaleString(locale)} 个输出`
    : `${count.toLocaleString(locale)} outputs`;
}
