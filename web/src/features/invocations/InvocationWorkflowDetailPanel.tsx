import { useEffect, useRef, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { Badge } from "../../components/ui/badge";
import { Spinner } from "../../components/ui/spinner";
import { useTranslation } from "../../i18n";
import type {
  ApiInvocation,
  ApiInvocationRequestBodyResponse,
  ApiInvocationResponseBodyResponse,
  ApiInvocationWorkflowDetailResponse,
  ApiInvocationWorkflowTimelineEntry,
} from "../../lib/api";
import {
  fetchInvocationRequestBody,
  fetchInvocationResponseBody,
  fetchInvocationWorkflowDetail,
} from "../../lib/api";
import {
  formatDashboardWorkingConversationSequenceId,
  hashDashboardWorkingConversationKey,
} from "../../lib/dashboardWorkingConversations";
import { resolveInvocationDisplayStatus } from "../../lib/invocationStatus";
import { cn } from "../../lib/utils";
import { AppIcon } from "../shared/AppIcon";
import { StructuredPayloadViewer } from "./StructuredPayloadViewer";

type DetailPanelSize = "compact" | "default";
type AttemptSection =
  | "timing"
  | "requestParsed"
  | "requestHeaders"
  | "requestBody"
  | "responseParsed"
  | "responseHeaders"
  | "responseBody";
type GenericSection = "json" | "body";

interface PayloadFetchState<T> {
  status: "idle" | "loading" | "loaded" | "error";
  data: T | null;
  error: string | null;
}

interface InvocationWorkflowDetailPanelProps {
  record: ApiInvocation;
  focusedAttemptId?: string | null;
  size?: DetailPanelSize;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
}

const FALLBACK_CELL = "—";

function formatDurationMs(value: number | null | undefined, locale: string) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  const seconds = value / 1000;
  const precision = Math.abs(seconds) >= 10 ? 1 : 2;
  return `${seconds.toLocaleString(locale, {
    minimumFractionDigits: 0,
    maximumFractionDigits: precision,
  })} s`;
}

function formatMilliseconds(value: number | null | undefined, locale: string) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  return `${value.toLocaleString(locale, {
    minimumFractionDigits: 0,
    maximumFractionDigits: 1,
  })} ms`;
}

function formatTimestamp(value: string | null | undefined, locale: string) {
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

function formatOptionalText(value: string | null | undefined) {
  const normalized = value?.trim();
  return normalized ? normalized : FALLBACK_CELL;
}

function formatOptionalNumber(value: number | null | undefined, locale: string) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  return value.toLocaleString(locale);
}

function formatCurrency(value: number | null | undefined, locale: string) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  return new Intl.NumberFormat(locale, {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 4,
    maximumFractionDigits: 4,
  }).format(value);
}

function buildConversationShortId(promptCacheKey: string | null | undefined) {
  const normalized = promptCacheKey?.trim();
  if (!normalized) return FALLBACK_CELL;
  return formatDashboardWorkingConversationSequenceId(
    `WC-${hashDashboardWorkingConversationKey(normalized).slice(0, 6)}`,
  );
}

function buildPayloadViewerLabels(isZh: boolean) {
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

function resolveStatusMeta(status: string | null | undefined, isZh: boolean) {
  const normalized = (status ?? "").trim().toLowerCase();
  if (normalized === "success" || normalized === "completed") {
    return { variant: "success" as const, label: isZh ? "成功" : "Success" };
  }
  if (normalized === "warning_success") {
    return { variant: "warning" as const, label: isZh ? "告警成功" : "Warning" };
  }
  if (normalized === "failed" || normalized === "transport_failure") {
    return { variant: "error" as const, label: isZh ? "失败" : "Failed" };
  }
  if (normalized === "http_failure") {
    return { variant: "error" as const, label: isZh ? "HTTP 失败" : "HTTP Failure" };
  }
  if (normalized === "budget_exhausted_final") {
    return {
      variant: "warning" as const,
      label: isZh ? "预算耗尽" : "Budget Exhausted",
    };
  }
  if (normalized === "running") {
    return { variant: "default" as const, label: isZh ? "运行中" : "Running" };
  }
  if (normalized === "pending") {
    return { variant: "secondary" as const, label: isZh ? "等待中" : "Pending" };
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

function resolveKindMeta(kind: string, isZh: boolean) {
  switch (kind) {
    case "routingDecision":
      return {
        label: isZh ? "路由" : "Route",
        variant: "secondary" as const,
        markerClass: "border-info/55 bg-info/18 text-info",
      };
    case "routingWait":
      return {
        label: isZh ? "等待" : "Wait",
        variant: "secondary" as const,
        markerClass: "border-accent/55 bg-accent/18 text-accent-content",
      };
    case "systemFinalFailure":
      return {
        label: isZh ? "裁定" : "Final",
        variant: "warning" as const,
        markerClass: "border-warning/70 bg-warning/25 text-warning-content",
      };
    default:
      return {
        label: isZh ? "尝试" : "Attempt",
        variant: "default" as const,
        markerClass: "border-primary/60 bg-primary/14 text-primary",
      };
  }
}

function formatRouteMode(value: string | null | undefined, isZh: boolean) {
  const normalized = value?.trim().toLowerCase();
  if (!normalized) return FALLBACK_CELL;
  if (normalized === "pool") return isZh ? "号池" : "Pool";
  if (normalized === "forward_proxy") return isZh ? "代理直连" : "Forward Proxy";
  if (normalized === "direct") return isZh ? "直连" : "Direct";
  return value?.trim() ?? FALLBACK_CELL;
}

function stringifyStructuredValue(value: Record<string, unknown> | null | undefined) {
  if (!value) return "";
  return JSON.stringify(value, null, 2);
}

function readString(value: unknown) {
  if (typeof value !== "string") return null;
  const normalized = value.trim();
  return normalized ? normalized : null;
}

function readNumber(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function readBoolean(value: unknown) {
  return typeof value === "boolean" ? value : null;
}

function readRecord(value: unknown) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
}

function readArray(value: unknown) {
  return Array.isArray(value) ? value : null;
}

function formatBooleanLabel(value: boolean | null | undefined, isZh: boolean) {
  if (value == null) return FALLBACK_CELL;
  return value ? (isZh ? "是" : "Yes") : isZh ? "否" : "No";
}

function formatUnknownValue(value: unknown, locale: string, isZh: boolean) {
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

function buildStructuredItems(
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

function normalizeToolLabel(value: unknown) {
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

function extractRequestBusinessSnapshot(bodyText: string) {
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

function extractResponseBusinessSnapshot(bodyText: string) {
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

function isRequestSection(section: AttemptSection) {
  return section === "requestParsed" || section === "requestHeaders" || section === "requestBody";
}

function isResponseSection(section: AttemptSection) {
  return (
    section === "responseParsed" || section === "responseHeaders" || section === "responseBody"
  );
}

interface TimelineMetricAction<TSection extends string> {
  section: TSection;
  label: string;
  tag?: string | null;
  primary: string;
  secondary?: string | null;
  tertiary?: string | null;
  tertiaryChips?: string[] | null;
  tertiaryOverflowCount?: number;
  monospace?: boolean;
}

function createIdlePayloadState<T>(): PayloadFetchState<T> {
  return {
    status: "idle",
    data: null,
    error: null,
  };
}

function formatHttpStatus(value: number | null | undefined, locale: string) {
  const status = formatOptionalNumber(value, locale);
  if (status === FALLBACK_CELL) return null;
  return `HTTP ${status}`;
}

function formatByteSize(value: number | null | undefined, locale: string) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  return `${value.toLocaleString(locale)} B`;
}

function compactJoin(parts: Array<string | null | undefined>) {
  const normalized = parts
    .map((part) => (typeof part === "string" ? part.trim() : ""))
    .filter((part) => part.length > 0 && part !== FALLBACK_CELL);
  return normalized.length > 0 ? normalized.join(" · ") : FALLBACK_CELL;
}

function formatHttpCompressionTag(value: string | null | undefined) {
  const normalized = value?.trim().toLowerCase();
  if (!normalized) return null;
  return normalized;
}

function formatCompactionSummary(value: string | null | undefined, isZh: boolean) {
  const normalized = value?.trim().toLowerCase();
  if (!normalized) return null;
  if (normalized === "remote_v2") return isZh ? "远程压缩V2" : "Remote compaction V2";
  if (normalized === "compact") return "Compact";
  return normalized;
}

function summarizeToolCalls(value: unknown, isZh: boolean) {
  const tools = readArray(value)
    ?.map((entry) => (typeof entry === "string" ? entry.trim() : ""))
    .filter(Boolean);
  if (!tools || tools.length === 0) return null;
  if (tools.length === 1) return tools[0];
  return isZh ? `${tools.length} 个工具` : `${tools.length} tools`;
}

function buildToolChips(value: unknown) {
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

function summarizeOutputItems(value: unknown, locale: string, isZh: boolean) {
  const count = readNumber(value);
  if (count == null) return null;
  return isZh
    ? `${count.toLocaleString(locale)} 个输出`
    : `${count.toLocaleString(locale)} outputs`;
}

function buildTimelineFacts(
  entry: ApiInvocationWorkflowTimelineEntry,
  isZh: boolean,
  localeTag: string,
) {
  const facts: string[] = [];
  if (entry.attempt) {
    const attempt = entry.attempt;
    const phase = formatOptionalText(attempt.phase);
    const upstreamStatus = formatHttpStatus(attempt.httpStatus, localeTag);
    const latencyValue =
      typeof attempt.streamLatencyMs === "number"
        ? `${isZh ? "流式" : "Stream"} ${formatDurationMs(attempt.streamLatencyMs, localeTag)}`
        : typeof attempt.firstByteLatencyMs === "number"
          ? `TTFB ${formatDurationMs(attempt.firstByteLatencyMs, localeTag)}`
          : null;

    if (attempt.upstreamAccountName?.trim()) facts.push(attempt.upstreamAccountName.trim());
    if (phase !== FALLBACK_CELL) facts.push(phase);
    if (upstreamStatus) facts.push(isZh ? `上游 ${upstreamStatus}` : `Upstream ${upstreamStatus}`);
    if (latencyValue) facts.push(latencyValue);
    if (attempt.synthetic) facts.push(isZh ? "合成尝试" : "Synthetic");
    return facts;
  }

  if (entry.subtitle?.trim()) facts.push(entry.subtitle.trim());

  const routeMode = formatRouteMode(readString(entry.detail?.routeMode), isZh);
  if (routeMode !== FALLBACK_CELL) facts.push(routeMode);

  const poolAttemptCount = readNumber(entry.detail?.poolAttemptCount);
  if (poolAttemptCount != null) {
    facts.push(isZh ? `尝试预算 ${poolAttemptCount}` : `Attempt budget ${poolAttemptCount}`);
  }

  const downstreamStatusCode = readNumber(entry.detail?.downstreamStatusCode);
  if (downstreamStatusCode != null)
    facts.push(`HTTP ${downstreamStatusCode.toLocaleString(localeTag)}`);

  const failureClass = readString(entry.detail?.failureClass);
  if (failureClass) facts.push(failureClass);

  return facts.filter(Boolean);
}

function buildAttemptMetricActions(
  entry: ApiInvocationWorkflowTimelineEntry,
  localeTag: string,
  isZh: boolean,
): Array<TimelineMetricAction<AttemptSection>> {
  const attempt = entry.attempt;
  if (!attempt) return [];
  const requestSummary = readRecord(attempt.requestSummary);
  const responseSummary = readRecord(attempt.responseSummary);
  const requestRouting = readRecord(requestSummary?.routing);
  const requestHeaders = readRecord(requestSummary?.headers);
  const requestCompression = readRecord(requestSummary?.compression);
  const requestBodyCapture = readRecord(requestSummary?.bodyCapture);
  const responseHeaders = readRecord(responseSummary?.headers);
  const responseBodyCapture = readRecord(responseSummary?.responseBodyCapture);
  const requestModel =
    readString(requestSummary?.requestModel) ?? formatOptionalText(attempt.requestModel);
  const responseModel =
    readString(requestSummary?.responseModel) ?? formatOptionalText(attempt.responseModel);
  const requesterIp = formatOptionalText(attempt.requesterIp);
  const responseStatus = readString(responseSummary?.status) ?? formatOptionalText(attempt.status);
  const responseHeadersMetric =
    formatHttpStatus(attempt.httpStatus, localeTag) ??
    formatOptionalText(readString(responseSummary?.responseContentEncoding));
  const requestTier = formatOptionalText(readString(requestSummary?.requestedServiceTier));
  const requestReasoning = formatOptionalText(readString(requestSummary?.reasoningEffort));
  const requestTransport = formatOptionalText(readString(requestSummary?.transport));
  const requestEndpoint = formatOptionalText(readString(requestSummary?.endpoint));
  const requestCompaction = formatCompactionSummary(
    readString(requestSummary?.compactionRequestKind),
    isZh,
  );
  const proxyDisplay = formatOptionalText(readString(requestRouting?.proxyDisplayName));
  const requestUserAgent = formatOptionalText(readString(requestHeaders?.userAgent));
  const requestBodySize = formatByteSize(readNumber(requestBodyCapture?.size), localeTag);
  const requestBodyDetail = formatOptionalText(readString(requestBodyCapture?.detailLevel));
  const responseFailureKind = formatOptionalText(
    readString(responseSummary?.failureKind) ?? attempt.failureKind ?? null,
  );
  const responseTier = formatOptionalText(readString(responseSummary?.serviceTier));
  const responseCompaction = formatCompactionSummary(
    readString(responseSummary?.compactionResponseKind),
    isZh,
  );
  const responseToolChips = buildToolChips(responseSummary?.toolCalls);
  const responseToolSummary =
    summarizeToolCalls(responseSummary?.toolCalls, isZh) ??
    summarizeOutputItems(responseSummary?.outputItems, localeTag, isZh);
  const responseBodySize = formatByteSize(readNumber(responseBodyCapture?.size), localeTag);
  const responseBodyDetail = formatOptionalText(readString(responseBodyCapture?.detailLevel));
  const responseOutputSummary = summarizeOutputItems(responseSummary?.outputItems, localeTag, isZh);
  const responseBodySummary =
    responseFailureKind !== FALLBACK_CELL
      ? responseFailureKind
      : (responseOutputSummary ?? (responseStatus !== FALLBACK_CELL ? responseStatus : null));
  const requestHttpCompressionTag = formatHttpCompressionTag(
    readString(requestCompression?.algorithm),
  );
  const responseHttpCompressionTag = formatHttpCompressionTag(
    readString(responseHeaders?.contentEncoding) ??
      readString(responseSummary?.responseContentEncoding),
  );

  const actions: Array<TimelineMetricAction<AttemptSection>> = [
    {
      section: "timing",
      label: isZh ? "时间" : "Timing",
      primary: formatDurationMs(
        typeof attempt.streamLatencyMs === "number"
          ? attempt.streamLatencyMs
          : typeof attempt.firstByteLatencyMs === "number"
            ? attempt.firstByteLatencyMs
            : attempt.connectLatencyMs,
        localeTag,
      ),
      secondary: `TTFB ${formatDurationMs(attempt.firstByteLatencyMs, localeTag)}`,
    },
    {
      section: "requestParsed",
      label: isZh ? "请求" : "Request",
      primary: requestModel,
      secondary: compactJoin([requestTier, requestReasoning]),
      tertiary: compactJoin([requestTransport, requestEndpoint]),
    },
    {
      section: "requestHeaders",
      label: isZh ? "请求头" : "Headers",
      primary: requesterIp,
      secondary: requestUserAgent,
      tertiary: proxyDisplay,
    },
    {
      section: "requestBody",
      label: isZh ? "请求体" : "Body",
      tag: requestHttpCompressionTag,
      primary: compactJoin([requestBodySize, requestBodyDetail]),
      secondary: requestCompaction,
      monospace: true,
    },
    {
      section: "responseParsed",
      label: isZh ? "响应" : "Response",
      primary: responseModel !== FALLBACK_CELL ? responseModel : responseStatus,
      secondary: compactJoin([responseTier, responseCompaction]),
      tertiary:
        responseToolChips && responseToolChips.visible.length > 0
          ? null
          : (responseToolSummary ?? responseFailureKind ?? responseStatus),
      tertiaryChips: responseToolChips?.visible ?? null,
      tertiaryOverflowCount: responseToolChips?.overflowCount ?? 0,
    },
    {
      section: "responseHeaders",
      label: isZh ? "响应头" : "Headers",
      primary: responseHeadersMetric ?? FALLBACK_CELL,
    },
    {
      section: "responseBody",
      label: isZh ? "响应体" : "Body",
      tag: responseHttpCompressionTag,
      primary: compactJoin([responseBodySize, responseBodyDetail]),
      secondary: responseBodySummary,
    },
  ];

  return actions.filter((item) => item.primary !== FALLBACK_CELL);
}

function buildGenericMetricActions(
  entry: ApiInvocationWorkflowTimelineEntry,
  localeTag: string,
  isZh: boolean,
): Array<TimelineMetricAction<GenericSection>> {
  const actions: Array<TimelineMetricAction<GenericSection>> = [];
  const routeMode = formatRouteMode(readString(entry.detail?.routeMode), isZh);
  const failureClass = formatOptionalText(readString(entry.detail?.failureClass));
  const downstreamStatusCode = readNumber(entry.detail?.downstreamStatusCode);
  const jsonMetric =
    routeMode !== FALLBACK_CELL
      ? routeMode
      : failureClass !== FALLBACK_CELL
        ? failureClass
        : formatOptionalText(entry.status);

  if (stringifyStructuredValue(entry.detail ?? undefined)) {
    actions.push({
      section: "json",
      label:
        entry.kind === "routingDecision"
          ? isZh
            ? "路由"
            : "Route"
          : entry.kind === "systemFinalFailure"
            ? isZh
              ? "裁定"
              : "Adjudication"
            : isZh
              ? "详情"
              : "Detail",
      primary: jsonMetric,
      secondary: compactJoin([
        formatOptionalText(entry.subtitle),
        typeof downstreamStatusCode === "number"
          ? `HTTP ${downstreamStatusCode.toLocaleString(localeTag)}`
          : null,
      ]),
    });
  }

  if (entry.responseBody) {
    actions.push({
      section: "body",
      label: isZh ? "返回体" : "Returned body",
      primary:
        typeof downstreamStatusCode === "number"
          ? `HTTP ${downstreamStatusCode.toLocaleString(localeTag)}`
          : entry.responseBody.available
            ? isZh
              ? "可用"
              : "Available"
            : formatOptionalText(entry.responseBody.unavailableReason),
      secondary: compactJoin([
        formatOptionalText(readString(entry.detail?.failureClass)),
        formatOptionalText(readString(entry.detail?.failureKind)),
      ]),
    });
  }

  return actions.filter((item) => item.primary !== FALLBACK_CELL);
}

function IdentityField({
  label,
  value,
  monospace = false,
}: {
  label: string;
  value: string;
  monospace?: boolean;
}) {
  return (
    <div className="min-w-0">
      <div className="text-[11px] font-medium text-base-content/56">{label}</div>
      <div
        className={cn(
          "mt-1 min-w-0 break-all text-sm text-base-content/84",
          monospace && "font-mono text-[13px]",
        )}
      >
        {value}
      </div>
    </div>
  );
}

function SummaryRows({
  rows,
}: {
  rows: Array<{
    label: string;
    value: string;
    variant?: "default" | "secondary" | "success" | "warning" | "error";
    action?: {
      title: string;
      onClick: () => void;
    };
  }>;
}) {
  return (
    <dl className="divide-y divide-base-300/62">
      {rows.map((row) => (
        <div key={row.label} className="flex items-start justify-between gap-4 py-3">
          <dt className="text-[11px] font-medium text-base-content/58">{row.label}</dt>
          <dd
            className={cn(
              "min-w-0 text-right text-sm font-medium text-base-content/88",
              row.variant === "success" && "text-success",
              row.variant === "warning" && "text-warning-content",
              row.variant === "error" && "text-error",
              row.variant === "default" && "text-info",
            )}
          >
            {row.action ? (
              <button
                type="button"
                title={row.action.title}
                className="break-all text-right underline decoration-dotted underline-offset-2 transition hover:text-primary"
                onClick={row.action.onClick}
              >
                {row.value}
              </button>
            ) : (
              <span className="break-all">{row.value}</span>
            )}
          </dd>
        </div>
      ))}
    </dl>
  );
}

function SnapshotMetric({
  label,
  value,
  variant = "secondary",
}: {
  label: string;
  value: string;
  variant?: "default" | "secondary" | "success" | "warning" | "error";
}) {
  return (
    <div className="rounded-[0.95rem] border border-base-300/68 bg-base-100/80 px-3 py-3 shadow-[0_10px_20px_rgba(15,23,42,0.035)]">
      <div className="text-[11px] font-medium text-base-content/56">{label}</div>
      <div
        className={cn(
          "mt-1 break-all text-sm font-semibold text-base-content",
          variant === "success" && "text-success",
          variant === "warning" && "text-warning-content",
          variant === "error" && "text-error",
          variant === "default" && "text-info",
        )}
      >
        {value}
      </div>
    </div>
  );
}

function OverviewGrid({
  items,
}: {
  items: Array<{ label: string; value: string; monospace?: boolean }>;
}) {
  return (
    <dl className="grid gap-x-5 gap-y-4 md:grid-cols-2 xl:grid-cols-3">
      {items.map((item) => (
        <div key={`${item.label}-${item.value}`} className="min-w-0">
          <dt className="text-[11px] font-medium text-base-content/58">{item.label}</dt>
          <dd
            className={cn(
              "mt-1 break-all text-sm text-base-content/86",
              item.monospace !== false && "font-mono",
            )}
          >
            {item.value}
          </dd>
        </div>
      ))}
    </dl>
  );
}

function DetailInfoPanel({
  title,
  items,
}: {
  title: string;
  items: Array<{ label: string; value: string; monospace?: boolean }>;
}) {
  if (items.length === 0) return null;
  return (
    <section className="rounded-[0.95rem] border border-base-300/68 bg-base-100/84 px-3.5 py-3">
      <div className="text-[11px] font-medium text-base-content/56">{title}</div>
      <div className="mt-3">
        <OverviewGrid items={items} />
      </div>
    </section>
  );
}

function DetailMetaStrip({
  items,
}: {
  items: Array<{
    label: string;
    value: string;
    monospace?: boolean;
    fullWidth?: boolean;
  }>;
}) {
  if (items.length === 0) return null;
  return (
    <section className="flex flex-wrap gap-x-4 gap-y-2 rounded-[0.95rem] border border-base-300/68 bg-base-100/84 px-3 py-2.5">
      {items.map((item) => (
        <div
          key={`${item.label}-${item.value}`}
          className={cn(
            "flex min-w-0 items-baseline gap-1.5 text-[12px] leading-5",
            item.fullWidth && "basis-full",
          )}
        >
          <span className="shrink-0 text-[10.5px] font-medium text-base-content/52">
            {item.label}
          </span>
          <span
            className={cn(
              "min-w-0 break-all font-semibold text-base-content/84",
              item.monospace && "font-mono text-[11.5px]",
            )}
          >
            {item.value}
          </span>
        </div>
      ))}
    </section>
  );
}

function DetailFrame({
  controls,
  children,
}: {
  controls?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-2.5 rounded-b-[1rem] border border-t-0 border-base-300/72 bg-base-100/72 px-3.5 pb-3.5 pt-2.5">
      {controls ? <div className="flex items-center justify-between gap-3">{controls}</div> : null}
      {children}
    </div>
  );
}

function TimelineMetricButton({
  label,
  tag,
  primary,
  secondary,
  tertiary,
  tertiaryChips,
  tertiaryOverflowCount = 0,
  monospace = false,
  active,
  onClick,
}: {
  label: string;
  tag?: string | null;
  primary: string;
  secondary?: string | null;
  tertiary?: string | null;
  tertiaryChips?: string[] | null;
  tertiaryOverflowCount?: number;
  monospace?: boolean;
  active?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className={cn(
        "h-full min-w-0 bg-base-100/84 px-3 py-2.5 text-left transition-[background-color,color] duration-150",
        active ? "bg-primary/8 text-primary" : "text-base-content hover:bg-base-100",
      )}
      onClick={onClick}
    >
      <div className="flex h-full flex-col gap-1">
        <div className="flex h-5 items-start justify-between gap-2">
          <div
            className={cn(
              "text-[10.5px] font-medium",
              active ? "text-primary" : "text-base-content/50",
            )}
          >
            {label}
          </div>
          {tag ? (
            <span
              title={tag}
              className={cn(
                "inline-flex shrink-0 items-center rounded-full border px-1.5 py-0.5 text-[9.5px] font-semibold leading-none",
                active
                  ? "border-primary/24 bg-primary/10 text-primary/82"
                  : "border-base-300/72 bg-base-200/86 text-base-content/54",
              )}
            >
              {tag}
            </span>
          ) : null}
        </div>
        <div className="flex min-h-0 flex-col gap-0.5">
          <div
            title={primary}
            className={cn(
              "overflow-hidden text-[12.5px] font-semibold leading-[1.3] [display:-webkit-box] [-webkit-box-orient:vertical] [-webkit-line-clamp:2] [overflow-wrap:anywhere]",
              monospace && "font-mono text-[12px] leading-[1.35]",
              active ? "text-primary" : "text-base-content/88",
            )}
          >
            {primary}
          </div>
          {secondary && secondary !== FALLBACK_CELL ? (
            <div
              title={secondary}
              className="overflow-hidden text-[10.5px] leading-4 text-base-content/64 text-ellipsis whitespace-nowrap"
            >
              {secondary}
            </div>
          ) : null}
          {tertiaryChips && tertiaryChips.length > 0 ? (
            <div className="flex min-w-0 items-center gap-1 overflow-hidden">
              {tertiaryChips.map((chip) => (
                <span
                  key={`${label}-${chip}`}
                  title={chip}
                  className={cn(
                    "inline-flex shrink-0 items-center rounded-full border px-1.5 py-0.5 text-[9.5px] font-medium leading-none",
                    active
                      ? "border-primary/18 bg-primary/10 text-primary/82"
                      : "border-base-300/72 bg-base-200/78 text-base-content/56",
                  )}
                >
                  {chip}
                </span>
              ))}
              {tertiaryOverflowCount > 0 ? (
                <span
                  className={cn(
                    "inline-flex shrink-0 items-center rounded-full border px-1.5 py-0.5 text-[9.5px] font-semibold leading-none",
                    active
                      ? "border-primary/18 bg-primary/10 text-primary/82"
                      : "border-base-300/72 bg-base-200/78 text-base-content/56",
                  )}
                >
                  +{tertiaryOverflowCount}
                </span>
              ) : null}
            </div>
          ) : tertiary && tertiary !== FALLBACK_CELL ? (
            <div
              title={tertiary}
              className="overflow-hidden text-[10.5px] leading-4 text-base-content/48 text-ellipsis whitespace-nowrap"
            >
              {tertiary}
            </div>
          ) : null}
        </div>
      </div>
    </button>
  );
}

function PayloadNotice({
  tone = "default",
  children,
}: {
  tone?: "default" | "warning" | "error";
  children: React.ReactNode;
}) {
  return (
    <div
      className={cn(
        "rounded-xl border px-3 py-3 text-sm",
        tone === "warning" && "border-warning/30 bg-warning/8 text-base-content/74",
        tone === "error" && "border-error/24 bg-error/6 text-base-content/78",
        tone === "default" && "border-base-300/70 bg-base-100/80 text-base-content/64",
      )}
    >
      {children}
    </div>
  );
}

function AttemptDetail({
  record,
  entry,
  localeTag,
  isZh,
  activeSection,
  requestBodyState,
  responseBodyState,
}: {
  record: ApiInvocation;
  entry: ApiInvocationWorkflowTimelineEntry;
  localeTag: string;
  isZh: boolean;
  activeSection: AttemptSection;
  requestBodyState: PayloadFetchState<ApiInvocationRequestBodyResponse>;
  responseBodyState: PayloadFetchState<ApiInvocationResponseBodyResponse>;
}) {
  const attempt = entry.attempt;
  if (!attempt) return null;
  const labels = buildPayloadViewerLabels(isZh);
  const requestSummary = readRecord(attempt.requestSummary);
  const responseSummary = readRecord(attempt.responseSummary);
  const requestBodyParsed = requestBodyState.data?.bodyText
    ? extractRequestBusinessSnapshot(requestBodyState.data.bodyText)
    : null;
  const responseBodyParsed = responseBodyState.data?.bodyText
    ? extractResponseBusinessSnapshot(responseBodyState.data.bodyText)
    : null;
  const requestHeaderSource =
    readRecord(requestSummary?.headers) ?? readRecord(requestBodyState.data?.headers);
  const requestRoutingSource =
    readRecord(requestSummary?.routing) ?? readRecord(requestBodyState.data?.routing);
  const requestClientSource =
    readRecord(requestSummary?.client) ??
    readRecord(readRecord(requestBodyState.data?.routing)?.client);
  const responseHeaderSource =
    readRecord(responseSummary?.headers) ?? readRecord(responseBodyState.data?.headers);
  const responseDeliverySource =
    readRecord(responseSummary?.delivery) ?? readRecord(responseBodyState.data?.routing);
  const requestBodyCaptureSource = readRecord(requestSummary?.bodyCapture);
  const requestArchiveAtInvocation = readBoolean(
    requestBodyCaptureSource?.availableAtInvocationLevel,
  );

  const keyDiagnosticsItems = [
    {
      label: isZh ? "账号" : "Account",
      value:
        formatOptionalText(attempt.upstreamAccountName) !== FALLBACK_CELL
          ? formatOptionalText(attempt.upstreamAccountName)
          : typeof attempt.upstreamAccountId === "number"
            ? `#${attempt.upstreamAccountId}`
            : FALLBACK_CELL,
      monospace: false,
    },
    { label: isZh ? "阶段" : "Phase", value: formatOptionalText(attempt.phase) },
    {
      label: isZh ? "上游 HTTP 状态" : "Upstream HTTP",
      value: formatHttpStatus(attempt.httpStatus, localeTag) ?? FALLBACK_CELL,
      monospace: false,
    },
    {
      label: isZh ? "连接耗时" : "Connect",
      value: formatMilliseconds(attempt.connectLatencyMs, localeTag),
    },
    {
      label: "TTFB",
      value: formatMilliseconds(attempt.firstByteLatencyMs, localeTag),
    },
    {
      label: isZh ? "流式耗时" : "Stream",
      value: formatMilliseconds(attempt.streamLatencyMs, localeTag),
    },
    {
      label: isZh ? "失败类型" : "Failure Kind",
      value: formatOptionalText(attempt.failureKind),
      monospace: false,
    },
    {
      label: isZh ? "上游请求 ID" : "Upstream Request ID",
      value: formatOptionalText(attempt.upstreamRequestId),
    },
  ].filter((item) => item.value !== FALLBACK_CELL);

  const timingItems = [
    {
      label: isZh ? "发生时间" : "Occurred At",
      value: formatTimestamp(attempt.occurredAt, localeTag),
      monospace: false,
    },
    {
      label: isZh ? "开始时间" : "Started At",
      value: formatTimestamp(attempt.startedAt, localeTag),
      monospace: false,
    },
    {
      label: isZh ? "结束时间" : "Finished At",
      value: formatTimestamp(attempt.finishedAt, localeTag),
      monospace: false,
    },
    {
      label: isZh ? "连接" : "Connect",
      value: formatMilliseconds(attempt.connectLatencyMs, localeTag),
    },
    { label: "TTFB", value: formatMilliseconds(attempt.firstByteLatencyMs, localeTag) },
    {
      label: isZh ? "流式" : "Stream",
      value: formatMilliseconds(attempt.streamLatencyMs, localeTag),
    },
    {
      label: isZh ? "读取请求" : "Request Read",
      value: formatMilliseconds(record.tReqReadMs, localeTag),
    },
    {
      label: isZh ? "解析请求" : "Request Parse",
      value: formatMilliseconds(record.tReqParseMs, localeTag),
    },
    {
      label: isZh ? "解析响应" : "Response Parse",
      value: formatMilliseconds(record.tRespParseMs, localeTag),
    },
    { label: isZh ? "持久化" : "Persist", value: formatMilliseconds(record.tPersistMs, localeTag) },
    { label: isZh ? "总用时" : "Total", value: formatMilliseconds(record.tTotalMs, localeTag) },
  ].filter((item) => item.value !== FALLBACK_CELL);

  const requestParsedItems = [
    ...buildStructuredItems(requestSummary, localeTag, isZh, [
      { key: "endpoint", label: isZh ? "端点" : "Endpoint", monospace: false },
      { key: "requestModel", label: isZh ? "请求模型" : "Request Model", monospace: false },
      { key: "responseModel", label: isZh ? "响应模型" : "Response Model", monospace: false },
      {
        key: "requestedServiceTier",
        label: isZh ? "请求服务层级" : "Requested Tier",
        monospace: false,
      },
      { key: "reasoningEffort", label: isZh ? "推理强度" : "Reasoning Effort", monospace: false },
      {
        key: "compactionRequestKind",
        label: isZh ? "请求压缩模式" : "Request Compaction",
        monospace: false,
      },
      { key: "imageIntent", label: isZh ? "图像工具意图" : "Image Intent", monospace: false },
      { key: "transport", label: isZh ? "传输" : "Transport", monospace: false },
    ]),
    ...buildStructuredItems(requestBodyParsed, localeTag, isZh, [
      { key: "model", label: isZh ? "请求体模型" : "Body Model", monospace: false },
      { key: "stream", label: isZh ? "流式返回" : "Streaming", monospace: false },
      { key: "serviceTier", label: isZh ? "请求体服务层级" : "Body Tier", monospace: false },
      {
        key: "reasoningEffort",
        label: isZh ? "请求体推理强度" : "Body Reasoning",
        monospace: false,
      },
      {
        key: "maxOutputTokens",
        label: isZh ? "最大输出 Token" : "Max Output Tokens",
        monospace: false,
      },
      { key: "temperature", label: isZh ? "温度" : "Temperature", monospace: false },
      { key: "topP", label: isZh ? "Top P" : "Top P", monospace: false },
      {
        key: "parallelToolCalls",
        label: isZh ? "并行工具调用" : "Parallel Tools",
        monospace: false,
      },
      { key: "toolChoice", label: isZh ? "工具选择" : "Tool Choice", monospace: false },
      { key: "tools", label: isZh ? "工具" : "Tools", monospace: false },
      { key: "modalities", label: isZh ? "模态" : "Modalities", monospace: false },
      { key: "inputShape", label: isZh ? "输入形态" : "Input Shape", monospace: false },
      { key: "inputCount", label: isZh ? "输入条目" : "Input Count", monospace: false },
      { key: "textFormat", label: isZh ? "返回格式" : "Response Format", monospace: false },
    ]),
  ];

  const requestHeaderItems = buildStructuredItems(requestHeaderSource, localeTag, isZh, [
    { key: "userAgent", label: "User-Agent", monospace: false },
    { key: "xForwardedFor", label: "X-Forwarded-For", monospace: false },
    { key: "forwarded", label: "Forwarded", monospace: false },
    { key: "xRealIp", label: "X-Real-IP", monospace: false },
  ]);

  const requestRoutingItems = [
    ...buildStructuredItems(requestRoutingSource, localeTag, isZh, [
      { key: "routeMode", label: isZh ? "路由模式" : "Route Mode", monospace: false },
      { key: "upstreamScope", label: isZh ? "上游范围" : "Upstream Scope", monospace: false },
      { key: "stickyKey", label: "Sticky Key" },
      { key: "promptCacheKey", label: isZh ? "Prompt Cache Key" : "Prompt Cache Key" },
      { key: "proxyDisplayName", label: isZh ? "代理显示名" : "Proxy Display", monospace: false },
      { key: "upstreamRouteKey", label: isZh ? "上游路由键" : "Upstream Route Key" },
      { key: "proxyBindingKey", label: isZh ? "代理绑定" : "Proxy Binding" },
      { key: "clientFingerprint", label: isZh ? "客户端指纹" : "Client Fingerprint" },
      {
        key: "oauthForwardedHeaderNames",
        label: isZh ? "OAuth 转发头" : "OAuth Forwarded Headers",
        monospace: false,
      },
      {
        key: "oauthPromptCacheHeaderForwarded",
        label: isZh ? "转发 Prompt Cache 头" : "Prompt Cache Header Forwarded",
        monospace: false,
      },
    ]),
    ...buildStructuredItems(requestClientSource, localeTag, isZh, [
      {
        key: "requestContainsEncryptedContent",
        label: isZh ? "请求含加密内容" : "Encrypted Request",
        monospace: false,
      },
      {
        key: "requestParseError",
        label: isZh ? "请求解析错误" : "Request Parse Error",
        monospace: false,
      },
      {
        key: "oauthAccountHeaderAttached",
        label: isZh ? "附带 OAuth 账号头" : "OAuth Account Header",
        monospace: false,
      },
      {
        key: "oauthAccountIdShape",
        label: isZh ? "OAuth 账号 ID 形态" : "OAuth Account ID Shape",
        monospace: false,
      },
      {
        key: "oauthRequestBodyPrefixFingerprint",
        label: isZh ? "请求体前缀指纹" : "Body Prefix Fingerprint",
        monospace: false,
      },
      {
        key: "oauthRequestBodyPrefixBytes",
        label: isZh ? "前缀字节数" : "Prefix Bytes",
        monospace: false,
      },
      {
        key: "oauthRequestBodySnapshotKind",
        label: isZh ? "请求体快照类型" : "Body Snapshot Kind",
        monospace: false,
      },
      {
        key: "oauthResponsesBodyMode",
        label: isZh ? "OAuth 响应体模式" : "OAuth Body Mode",
        monospace: false,
      },
      {
        key: "oauthResponsesRewrite",
        label: isZh ? "OAuth 改写" : "OAuth Rewrite",
        monospace: false,
      },
    ]),
  ];

  const requestCaptureSummaryItems = [
    {
      label: isZh ? "归档" : "Archive",
      value:
        requestArchiveAtInvocation == null
          ? FALLBACK_CELL
          : requestArchiveAtInvocation
            ? isZh
              ? "调用级"
              : "Invocation"
            : isZh
              ? "未存档"
              : "Unavailable",
      monospace: false,
    },
    {
      label: isZh ? "来源" : "Source",
      value: formatOptionalText(requestBodyState.data?.captureSource),
      monospace: false,
    },
    {
      label: isZh ? "大小" : "Size",
      value: formatByteSize(requestBodyState.data?.bodySize, localeTag),
      monospace: false,
    },
    {
      label: isZh ? "详情" : "Detail",
      value: formatOptionalText(requestBodyState.data?.detailLevel),
      monospace: false,
    },
    {
      label: isZh ? "截断" : "Truncated",
      value:
        requestBodyState.data?.bodyTruncated == null
          ? FALLBACK_CELL
          : requestBodyState.data.bodyTruncated
            ? isZh
              ? "已截断"
              : "Truncated"
            : isZh
              ? "未截断"
              : "Full",
      monospace: false,
    },
    {
      label: isZh ? "截断原因" : "Truncate Reason",
      value: formatOptionalText(requestBodyState.data?.bodyTruncatedReason),
      monospace: false,
      fullWidth: true,
    },
    {
      label: isZh ? "裁剪原因" : "Prune Reason",
      value: formatOptionalText(requestBodyState.data?.detailPruneReason),
      monospace: false,
      fullWidth: true,
    },
  ].filter((item) => item.value !== FALLBACK_CELL);

  const responseParsedItems = [
    ...buildStructuredItems(responseSummary, localeTag, isZh, [
      { key: "status", label: isZh ? "尝试状态" : "Attempt Status", monospace: false },
      { key: "phase", label: isZh ? "阶段" : "Phase", monospace: false },
      { key: "failureKind", label: isZh ? "失败类型" : "Failure Kind", monospace: false },
      { key: "errorMessage", label: isZh ? "错误信息" : "Error Message", monospace: false },
      {
        key: "downstreamErrorMessage",
        label: isZh ? "下游错误" : "Downstream Error",
        monospace: false,
      },
      { key: "serviceTier", label: isZh ? "服务层级" : "Service Tier", monospace: false },
      { key: "billingServiceTier", label: isZh ? "计费层级" : "Billing Tier", monospace: false },
      {
        key: "streamTerminalEvent",
        label: isZh ? "流终止事件" : "Stream Terminal",
        monospace: false,
      },
      {
        key: "responseContentEncoding",
        label: isZh ? "响应编码" : "Response Encoding",
        monospace: false,
      },
      {
        key: "compactionResponseKind",
        label: isZh ? "响应压缩模式" : "Response Compaction",
        monospace: false,
      },
    ]),
    ...buildStructuredItems(responseBodyParsed, localeTag, isZh, [
      { key: "id", label: isZh ? "响应 ID" : "Response ID", monospace: false },
      { key: "object", label: isZh ? "对象类型" : "Object", monospace: false },
      { key: "status", label: isZh ? "响应状态" : "Body Status", monospace: false },
      { key: "model", label: isZh ? "响应体模型" : "Body Model", monospace: false },
      { key: "serviceTier", label: isZh ? "响应体服务层级" : "Body Tier", monospace: false },
      { key: "outputItems", label: isZh ? "输出项" : "Output Items", monospace: false },
      { key: "outputTextBlocks", label: isZh ? "文本块" : "Output Text Blocks", monospace: false },
      { key: "toolCalls", label: isZh ? "工具调用" : "Tool Calls", monospace: false },
      { key: "errorCode", label: isZh ? "错误码" : "Error Code", monospace: false },
      { key: "errorMessage", label: isZh ? "错误消息" : "Error Message", monospace: false },
      { key: "usageInputTokens", label: isZh ? "输入 Token" : "Input Tokens", monospace: false },
      { key: "usageOutputTokens", label: isZh ? "输出 Token" : "Output Tokens", monospace: false },
      {
        key: "usageReasoningTokens",
        label: isZh ? "推理 Token" : "Reasoning Tokens",
        monospace: false,
      },
      { key: "usageTotalTokens", label: isZh ? "总 Token" : "Total Tokens", monospace: false },
    ]),
  ];

  const responseHeaderItems = buildStructuredItems(responseHeaderSource, localeTag, isZh, [
    { key: "contentEncoding", label: "Content-Encoding", monospace: false },
    { key: "contentEncodingChain", label: isZh ? "编码链" : "Encoding Chain", monospace: false },
    { key: "upstreamRequestId", label: "X-Request-ID", monospace: false },
    { key: "cvmInvokeId", label: isZh ? "CVM 调用 ID" : "CVM Invoke ID" },
  ]);

  const responseDeliveryItems = buildStructuredItems(responseDeliverySource, localeTag, isZh, [
    { key: "forwardedChunkCount", label: isZh ? "转发块数" : "Forwarded Chunks", monospace: false },
    { key: "forwardedBytes", label: isZh ? "转发字节" : "Forwarded Bytes", monospace: false },
    { key: "usageObserved", label: isZh ? "观察到 Usage" : "Usage Observed", monospace: false },
    {
      key: "downstreamClosePhase",
      label: isZh ? "下游关闭阶段" : "Downstream Close Phase",
      monospace: false,
    },
    {
      key: "downstreamWriteErrorKind",
      label: isZh ? "下游写错误" : "Downstream Write Error",
      monospace: false,
    },
    {
      key: "lastUpstreamChunkGapMs",
      label: isZh ? "最后块间隔" : "Last Chunk Gap",
      monospace: false,
    },
    {
      key: "streamFailureOrigin",
      label: isZh ? "流失败来源" : "Stream Failure Origin",
      monospace: false,
    },
    {
      key: "upstreamReadErrorKind",
      label: isZh ? "上游读取错误" : "Upstream Read Error",
      monospace: false,
    },
    {
      key: "responseContainsEncryptedContent",
      label: isZh ? "响应含加密内容" : "Encrypted Response",
      monospace: false,
    },
  ]);

  const responseCaptureSummaryItems = [
    {
      label: isZh ? "归档" : "Archive",
      value: isZh ? "调用级" : "Invocation",
      monospace: false,
    },
    {
      label: isZh ? "来源" : "Source",
      value: formatOptionalText(responseBodyState.data?.captureSource),
      monospace: false,
    },
    {
      label: isZh ? "大小" : "Size",
      value: formatByteSize(responseBodyState.data?.bodySize, localeTag),
      monospace: false,
    },
    {
      label: isZh ? "详情" : "Detail",
      value: formatOptionalText(responseBodyState.data?.detailLevel),
      monospace: false,
    },
    {
      label: isZh ? "截断" : "Truncated",
      value:
        responseBodyState.data?.bodyTruncated == null
          ? FALLBACK_CELL
          : responseBodyState.data.bodyTruncated
            ? isZh
              ? "已截断"
              : "Truncated"
            : isZh
              ? "未截断"
              : "Full",
      monospace: false,
    },
    {
      label: isZh ? "截断原因" : "Truncate Reason",
      value: formatOptionalText(responseBodyState.data?.bodyTruncatedReason),
      monospace: false,
      fullWidth: true,
    },
    {
      label: isZh ? "裁剪原因" : "Prune Reason",
      value: formatOptionalText(responseBodyState.data?.detailPruneReason),
      monospace: false,
      fullWidth: true,
    },
  ].filter((item) => item.value !== FALLBACK_CELL);

  const requestBodyContent = requestBodyState.data?.bodyText?.trim() ?? "";
  const responseBodyContent = responseBodyState.data?.bodyText?.trim() ?? "";

  return (
    <DetailFrame>
      {activeSection === "timing" ? (
        <>
          <DetailInfoPanel
            title={isZh ? "关键诊断" : "Key diagnostics"}
            items={keyDiagnosticsItems}
          />
          <DetailInfoPanel title={isZh ? "时间细分" : "Timing breakdown"} items={timingItems} />
        </>
      ) : null}

      {activeSection === "requestParsed" ? (
        <>
          <DetailInfoPanel
            title={isZh ? "解析后的请求" : "Parsed request"}
            items={requestParsedItems}
          />
          <DetailInfoPanel
            title={isZh ? "路由与会话信号" : "Routing and session"}
            items={requestRoutingItems}
          />
          {requestBodyState.status === "loading" ? (
            <PayloadNotice>{isZh ? "加载请求体…" : "Loading request body…"}</PayloadNotice>
          ) : null}
          {requestBodyState.status === "error" ? (
            <PayloadNotice tone="error">
              {isZh ? "请求体加载失败：" : "Failed to load request body: "}
              {requestBodyState.error}
            </PayloadNotice>
          ) : null}
        </>
      ) : null}
      {activeSection === "requestHeaders" ? (
        <>
          <DetailInfoPanel title={isZh ? "请求头" : "Request headers"} items={requestHeaderItems} />
          <DetailInfoPanel
            title={isZh ? "路由与会话信号" : "Routing and session"}
            items={requestRoutingItems}
          />
          <DetailMetaStrip items={requestCaptureSummaryItems} />
        </>
      ) : null}
      {activeSection === "requestBody" ? (
        <>
          <DetailMetaStrip items={requestCaptureSummaryItems} />
          {requestBodyState.status === "loading" ? (
            <PayloadNotice>{isZh ? "加载请求体…" : "Loading request body…"}</PayloadNotice>
          ) : requestBodyState.status === "error" ? (
            <PayloadNotice tone="error">
              {isZh ? "请求体加载失败：" : "Failed to load request body: "}
              {requestBodyState.error}
            </PayloadNotice>
          ) : requestBodyState.data?.available && requestBodyContent ? (
            <StructuredPayloadViewer value={requestBodyContent} labels={labels} />
          ) : (
            <PayloadNotice tone="warning">
              {isZh ? "请求体不可用：" : "Request body unavailable: "}
              {requestBodyState.data?.unavailableReason ?? FALLBACK_CELL}
            </PayloadNotice>
          )}
        </>
      ) : null}
      {activeSection === "responseParsed" ? (
        <>
          <DetailInfoPanel
            title={isZh ? "解析后的响应" : "Parsed response"}
            items={responseParsedItems}
          />
          <DetailInfoPanel
            title={isZh ? "传输与下游收口" : "Delivery and downstream"}
            items={responseDeliveryItems}
          />
          {responseBodyState.status === "loading" ? (
            <PayloadNotice>{isZh ? "加载响应体…" : "Loading response body…"}</PayloadNotice>
          ) : null}
          {responseBodyState.status === "error" ? (
            <PayloadNotice tone="error">
              {isZh ? "响应体加载失败：" : "Failed to load response body: "}
              {responseBodyState.error}
            </PayloadNotice>
          ) : null}
        </>
      ) : null}
      {activeSection === "responseHeaders" ? (
        <>
          <DetailInfoPanel
            title={isZh ? "响应头" : "Response headers"}
            items={responseHeaderItems}
          />
          <DetailInfoPanel
            title={isZh ? "传输与下游收口" : "Delivery and downstream"}
            items={responseDeliveryItems}
          />
          <DetailMetaStrip items={responseCaptureSummaryItems} />
        </>
      ) : null}
      {activeSection === "responseBody" ? (
        <>
          <DetailMetaStrip items={responseCaptureSummaryItems} />
          {responseBodyState.status === "loading" ? (
            <PayloadNotice>{isZh ? "加载响应体…" : "Loading response body…"}</PayloadNotice>
          ) : responseBodyState.status === "error" ? (
            <PayloadNotice tone="error">
              {isZh ? "响应体加载失败：" : "Failed to load response body: "}
              {responseBodyState.error}
            </PayloadNotice>
          ) : responseBodyState.data?.available && responseBodyContent ? (
            <StructuredPayloadViewer value={responseBodyContent} labels={labels} />
          ) : (
            <PayloadNotice tone="warning">
              {isZh ? "响应体不可用：" : "Response body unavailable: "}
              {responseBodyState.data?.unavailableReason ?? FALLBACK_CELL}
            </PayloadNotice>
          )}
        </>
      ) : null}
    </DetailFrame>
  );
}

function GenericDetail({
  entry,
  isZh,
  activeSection,
}: {
  entry: ApiInvocationWorkflowTimelineEntry;
  isZh: boolean;
  activeSection: GenericSection;
}) {
  const labels = buildPayloadViewerLabels(isZh);
  const detailContent = stringifyStructuredValue(entry.detail ?? undefined);
  const bodyText = entry.responseBody?.bodyText?.trim() ?? "";
  return (
    <DetailFrame>
      {activeSection === "json" ? (
        detailContent ? (
          <StructuredPayloadViewer value={detailContent} labels={labels} />
        ) : (
          <div className="rounded-xl border border-base-300/70 bg-base-100/80 px-3 py-3 text-sm text-base-content/62">
            {isZh ? "无 JSON" : "No JSON"}
          </div>
        )
      ) : null}

      {activeSection === "body" && entry.responseBody ? (
        entry.responseBody.available && bodyText ? (
          <StructuredPayloadViewer value={bodyText} labels={labels} />
        ) : (
          <div className="rounded-xl border border-warning/25 bg-warning/8 px-3 py-3 text-sm text-base-content/72">
            {isZh ? "响应体不可用：" : "Response body unavailable: "}
            {entry.responseBody.unavailableReason ?? FALLBACK_CELL}
          </div>
        )
      ) : null}
    </DetailFrame>
  );
}

function TimelineSummary({
  entry,
  localeTag,
  isZh,
  isOpen,
  activeSection,
  onSelectSection,
}: {
  entry: ApiInvocationWorkflowTimelineEntry;
  localeTag: string;
  isZh: boolean;
  isOpen: boolean;
  activeSection: AttemptSection | GenericSection | null;
  onSelectSection: (section: AttemptSection | GenericSection) => void;
}) {
  const kindMeta = resolveKindMeta(entry.kind, isZh);
  const statusMeta = resolveStatusMeta(entry.status, isZh);
  const summaryFacts = buildTimelineFacts(entry, isZh, localeTag);
  const attemptId = entry.attempt?.attemptId?.trim() || null;
  const showTitle = !entry.attempt;
  const metricActions = entry.attempt
    ? buildAttemptMetricActions(entry, localeTag, isZh)
    : buildGenericMetricActions(entry, localeTag, isZh);

  return (
    <div
      className={cn(
        "w-full rounded-[1rem] border px-4 py-3 text-left transition-[background-color,border-color,box-shadow] duration-200",
        isOpen
          ? "border-primary/32 bg-base-100/88 shadow-[0_12px_26px_rgba(15,23,42,0.07)]"
          : "border-base-300/72 bg-base-100/72 hover:border-base-300 hover:bg-base-100/88",
      )}
    >
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <Badge variant={kindMeta.variant}>{kindMeta.label}</Badge>
            {entry.status ? <Badge variant={statusMeta.variant}>{statusMeta.label}</Badge> : null}
            {attemptId ? (
              <span className="font-mono text-[11px] text-primary/90">{attemptId}</span>
            ) : null}
          </div>
          <div className={cn("min-w-0", showTitle ? "mt-2" : "mt-1.5")}>
            {showTitle ? (
              <div className="text-sm font-semibold text-base-content">{entry.title}</div>
            ) : null}
            {summaryFacts.length > 0 ? (
              <div
                className={cn(
                  "flex min-w-0 flex-wrap items-center gap-1.5 text-xs text-base-content/64",
                  showTitle ? "mt-1" : "mt-0.5",
                )}
              >
                {summaryFacts.map((fact) => (
                  <span
                    key={`${entry.blockId}-${fact}`}
                    className="min-w-0 break-all rounded-full bg-base-200/82 px-2 py-0.5"
                  >
                    {fact}
                  </span>
                ))}
              </div>
            ) : null}
          </div>
        </div>
        <div className="flex shrink-0 items-start gap-3">
          <div className="text-right text-xs text-base-content/58">
            {formatTimestamp(entry.occurredAt, localeTag)}
          </div>
          {isOpen ? (
            <AppIcon
              name="chevron-down"
              className="mt-0.5 h-4 w-4 text-base-content/52"
              aria-hidden
            />
          ) : null}
        </div>
      </div>
      {metricActions.length > 0 ? (
        <div className="mt-3 overflow-hidden rounded-[0.95rem] border border-base-300/72 bg-base-300/72">
          <div className="grid gap-px sm:grid-cols-2 xl:grid-cols-4 2xl:grid-cols-7">
            {metricActions.map((action) => (
              <TimelineMetricButton
                key={`${entry.blockId}-${action.section}`}
                label={action.label}
                tag={action.tag}
                primary={action.primary}
                secondary={action.secondary}
                tertiary={action.tertiary}
                tertiaryChips={action.tertiaryChips}
                tertiaryOverflowCount={action.tertiaryOverflowCount}
                monospace={action.monospace}
                active={isOpen && activeSection === action.section}
                onClick={() => onSelectSection(action.section)}
              />
            ))}
          </div>
        </div>
      ) : null}
    </div>
  );
}

export function InvocationWorkflowDetailPanel({
  record,
  focusedAttemptId = null,
  size = "default",
  onOpenUpstreamAccount,
}: InvocationWorkflowDetailPanelProps) {
  const { locale } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const isZh = locale === "zh";
  const requestSeqRef = useRef(0);
  const [detail, setDetail] = useState<ApiInvocationWorkflowDetailResponse | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [openBlockId, setOpenBlockId] = useState<string | null>(null);
  const [attemptSection, setAttemptSection] = useState<AttemptSection | null>(null);
  const [genericSection, setGenericSection] = useState<GenericSection | null>(null);
  const [requestBodyState, setRequestBodyState] = useState<
    PayloadFetchState<ApiInvocationRequestBodyResponse>
  >(createIdlePayloadState());
  const [responseBodyState, setResponseBodyState] = useState<
    PayloadFetchState<ApiInvocationResponseBodyResponse>
  >(createIdlePayloadState());

  useEffect(() => {
    if (!(record.id > 0)) {
      requestSeqRef.current += 1;
      setDetail(null);
      setIsLoading(false);
      setLoadError(null);
      setOpenBlockId(null);
      return;
    }

    const requestSeq = requestSeqRef.current + 1;
    requestSeqRef.current = requestSeq;
    setIsLoading(true);
    setLoadError(null);
    setRequestBodyState(createIdlePayloadState());
    setResponseBodyState(createIdlePayloadState());

    void fetchInvocationWorkflowDetail(record.id)
      .then((response) => {
        if (requestSeq !== requestSeqRef.current) return;
        setDetail(response);
      })
      .catch((error) => {
        if (requestSeq !== requestSeqRef.current) return;
        setLoadError(error instanceof Error ? error.message : String(error));
        setDetail(null);
      })
      .finally(() => {
        if (requestSeq === requestSeqRef.current) {
          setIsLoading(false);
        }
      });
  }, [record.id]);

  useEffect(() => {
    if (!detail) {
      setOpenBlockId(null);
      setAttemptSection(null);
      setGenericSection(null);
      return;
    }
    setOpenBlockId(null);
    setAttemptSection(null);
    setGenericSection(null);
    setRequestBodyState(createIdlePayloadState());
    setResponseBodyState(createIdlePayloadState());
  }, [detail, focusedAttemptId]);

  useEffect(() => {
    if (!(record.id > 0)) return;
    if (attemptSection && isRequestSection(attemptSection) && requestBodyState.status === "idle") {
      let cancelled = false;
      setRequestBodyState({ status: "loading", data: null, error: null });
      void fetchInvocationRequestBody(record.id)
        .then((data) => {
          if (!cancelled) {
            setRequestBodyState({ status: "loaded", data, error: null });
          }
        })
        .catch((error) => {
          if (!cancelled) {
            setRequestBodyState({
              status: "error",
              data: null,
              error: error instanceof Error ? error.message : String(error),
            });
          }
        });
      return () => {
        cancelled = true;
      };
    }
  }, [attemptSection, record.id, requestBodyState.status]);

  useEffect(() => {
    if (!(record.id > 0)) return;
    if (
      attemptSection &&
      isResponseSection(attemptSection) &&
      responseBodyState.status === "idle"
    ) {
      let cancelled = false;
      setResponseBodyState({ status: "loading", data: null, error: null });
      void fetchInvocationResponseBody(record.id)
        .then((data) => {
          if (!cancelled) {
            setResponseBodyState({ status: "loaded", data, error: null });
          }
        })
        .catch((error) => {
          if (!cancelled) {
            setResponseBodyState({
              status: "error",
              data: null,
              error: error instanceof Error ? error.message : String(error),
            });
          }
        });
      return () => {
        cancelled = true;
      };
    }
  }, [attemptSection, record.id, responseBodyState.status]);

  if (!(record.id > 0)) {
    return (
      <div className="rounded-[1rem] border border-base-300/72 bg-base-100/72 px-4 py-3 text-sm text-base-content/64">
        {isZh ? "调用未落盘" : "Invocation not persisted"}
      </div>
    );
  }

  if (isLoading && !detail) {
    return (
      <div className="flex min-h-40 items-center justify-center rounded-[1rem] border border-base-300/72 bg-base-100/72">
        <Spinner size="lg" />
      </div>
    );
  }

  if (loadError) {
    return (
      <Alert variant="error">
        {isZh ? "详情加载失败：" : "Detail load failed: "}
        {loadError}
      </Alert>
    );
  }

  if (!detail) {
    return null;
  }

  const hero = detail.hero;
  const timeline = detail.timeline;
  const conversationShortId = buildConversationShortId(hero.promptCacheKey);
  const finalStatusRaw =
    hero.finalStatus ?? resolveInvocationDisplayStatus(record) ?? record.status ?? FALLBACK_CELL;
  const finalStatusMeta = resolveStatusMeta(finalStatusRaw, isZh);
  const finalAccountLabel =
    formatOptionalText(hero.upstreamAccountName ?? record.upstreamAccountName) !== FALLBACK_CELL
      ? formatOptionalText(hero.upstreamAccountName ?? record.upstreamAccountName)
      : typeof (hero.upstreamAccountId ?? record.upstreamAccountId) === "number"
        ? `#${hero.upstreamAccountId ?? record.upstreamAccountId}`
        : FALLBACK_CELL;
  const finalAccountId = hero.upstreamAccountId ?? record.upstreamAccountId;
  const summaryRows = [
    {
      label: isZh ? "最终账号" : "Final Account",
      value: finalAccountLabel,
      action:
        onOpenUpstreamAccount &&
        typeof finalAccountId === "number" &&
        finalAccountLabel !== FALLBACK_CELL
          ? {
              title: finalAccountLabel,
              onClick: () => onOpenUpstreamAccount(finalAccountId, finalAccountLabel),
            }
          : undefined,
    },
    {
      label: isZh ? "下游状态" : "Downstream Status",
      value:
        typeof hero.downstreamStatusCode === "number"
          ? `HTTP ${hero.downstreamStatusCode.toLocaleString(localeTag)}`
          : FALLBACK_CELL,
    },
    {
      label: isZh ? "失败类" : "Failure Class",
      value: formatOptionalText(hero.failureClass ?? record.failureClass),
    },
    {
      label: isZh ? "尝试预算" : "Attempt Budget",
      value: formatOptionalNumber(hero.poolAttemptCount ?? record.poolAttemptCount, localeTag),
    },
    {
      label: isZh ? "总 Token" : "Total Tokens",
      value: formatOptionalNumber(hero.totalTokens, localeTag),
    },
    {
      label: isZh ? "成本" : "Cost",
      value: formatCurrency(hero.cost, localeTag),
    },
  ];
  const heroStatusNotes = [
    detail.reconstructed
      ? isZh
        ? "时间线由历史记录重建"
        : "Timeline reconstructed from stored records"
      : null,
    detail.partial
      ? `${isZh ? "信息不完整" : "Partial detail"}${detail.partialReason ? `: ${detail.partialReason}` : ""}`
      : null,
  ].filter((value): value is string => Boolean(value));
  const snapshotMetrics = [
    {
      label: isZh ? "最终结果" : "Final Result",
      value: finalStatusMeta.label,
      variant: finalStatusMeta.variant,
    },
    {
      label: isZh ? "总用时" : "Total Time",
      value: formatDurationMs(hero.totalDurationMs ?? record.tTotalMs, localeTag),
      variant: "default" as const,
    },
    {
      label: isZh ? "尝试次数" : "Attempts",
      value: formatOptionalNumber(hero.timelineAttemptCount ?? timeline.length, localeTag),
      variant: "secondary" as const,
    },
    {
      label: isZh ? "路由模式" : "Route Mode",
      value: formatRouteMode(hero.routeMode, isZh),
      variant: "secondary" as const,
    },
  ];

  const modelTrail = [
    formatOptionalText(hero.requestModel),
    formatOptionalText(hero.responseModel),
  ].filter((value) => value !== FALLBACK_CELL);

  const toggleAttemptSection = (
    entry: ApiInvocationWorkflowTimelineEntry,
    section: AttemptSection,
  ) => {
    if (openBlockId === entry.blockId && attemptSection === section) {
      setOpenBlockId(null);
      setAttemptSection(null);
      return;
    }
    setOpenBlockId(entry.blockId);
    setAttemptSection(section);
    setGenericSection(null);
  };

  const toggleGenericSection = (
    entry: ApiInvocationWorkflowTimelineEntry,
    section: GenericSection,
  ) => {
    if (openBlockId === entry.blockId && genericSection === section) {
      setOpenBlockId(null);
      setGenericSection(null);
      return;
    }
    setOpenBlockId(entry.blockId);
    setGenericSection(section);
    setAttemptSection(null);
  };

  return (
    <div className={cn("space-y-4", size === "compact" ? "text-sm" : "")}>
      <section className="rounded-[1.2rem] border border-primary/18 bg-[linear-gradient(180deg,rgba(255,255,255,0.95),rgba(239,246,255,0.88))] px-4 py-4 shadow-[0_18px_40px_rgba(15,23,42,0.06)] sm:px-5 sm:py-5">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div className="min-w-0">
            <div className="text-[11px] font-semibold uppercase tracking-[0.22em] text-primary/72">
              {isZh ? "调用详情" : "Invocation Detail"}
            </div>
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <Badge variant={finalStatusMeta.variant}>{finalStatusMeta.label}</Badge>
              {hero.routeMode ? (
                <Badge variant="secondary">{formatRouteMode(hero.routeMode, isZh)}</Badge>
              ) : null}
              {detail.reconstructed ? (
                <Badge variant="warning">{isZh ? "重建" : "Reconstructed"}</Badge>
              ) : null}
              {detail.partial ? <Badge variant="warning">{isZh ? "部分" : "Partial"}</Badge> : null}
            </div>
          </div>
          <div className="text-right">
            <div className="text-[11px] font-medium text-base-content/54">
              {isZh ? "调用时间" : "Occurred At"}
            </div>
            <div className="mt-1 text-sm font-medium text-base-content/78">
              {formatTimestamp(hero.occurredAt, localeTag)}
            </div>
          </div>
        </div>

        <div
          className={cn(
            "mt-4 grid gap-4",
            size === "compact"
              ? "xl:grid-cols-[minmax(0,1.4fr)_minmax(19rem,0.9fr)]"
              : "xl:grid-cols-[minmax(0,1.45fr)_minmax(22rem,0.95fr)]",
          )}
        >
          <div className="rounded-[1rem] border border-primary/16 bg-base-100/78 p-4 shadow-[0_14px_28px_rgba(15,23,42,0.04)]">
            <div className="grid gap-4 lg:grid-cols-[minmax(0,1.1fr)_minmax(14rem,0.9fr)]">
              <div className="min-w-0">
                <div className="text-[11px] font-medium text-base-content/56">
                  {isZh ? "调用 ID" : "Call ID"}
                </div>
                <div className="mt-1 break-all font-mono text-[1.08rem] font-semibold tracking-[-0.02em] text-base-content sm:text-[1.22rem]">
                  {hero.invokeId ?? record.invokeId}
                </div>
              </div>

              <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-1">
                <IdentityField
                  label={isZh ? "对话 ID" : "Conversation ID"}
                  value={conversationShortId}
                />
                <IdentityField
                  label={isZh ? "原始 Prompt Cache Key" : "Raw Prompt Cache Key"}
                  value={hero.promptCacheKey ?? FALLBACK_CELL}
                  monospace
                />
              </div>
            </div>

            <div className="mt-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
              {snapshotMetrics.map((metric) => (
                <SnapshotMetric
                  key={metric.label}
                  label={metric.label}
                  value={metric.value}
                  variant={metric.variant}
                />
              ))}
            </div>

            <div className="mt-4 grid gap-4 border-t border-base-300/65 pt-4 lg:grid-cols-[minmax(0,1fr)_minmax(16rem,0.95fr)]">
              <div className="min-w-0">
                <div className="text-[11px] font-medium text-base-content/56">
                  {isZh ? "模型与端点" : "Models and Endpoint"}
                </div>
                <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-sm text-base-content/84">
                  {modelTrail.length > 0 ? (
                    <>
                      {modelTrail.map((modelValue, index) => (
                        <span key={`${modelValue}-${index}`} className="min-w-0 break-all">
                          {modelValue}
                        </span>
                      ))}
                      <span className="text-base-content/46">·</span>
                    </>
                  ) : null}
                  <span className="break-all">{formatOptionalText(hero.endpoint)}</span>
                </div>
              </div>

              {heroStatusNotes.length > 0 ? (
                <div className="min-w-0">
                  <div className="text-[11px] font-medium text-base-content/56">
                    {isZh ? "状态" : "Status"}
                  </div>
                  <div className="mt-1 space-y-1 text-sm text-base-content/72">
                    {heroStatusNotes.map((note) => (
                      <div key={note}>{note}</div>
                    ))}
                  </div>
                </div>
              ) : (
                <div className="min-w-0" />
              )}
            </div>
          </div>

          <div className="rounded-[1rem] border border-base-300/72 bg-base-100/84 p-4 shadow-[0_14px_28px_rgba(15,23,42,0.04)]">
            <div className="flex items-center justify-between gap-3">
              <div className="text-sm font-semibold text-base-content">
                {isZh ? "关键指标" : "Key metrics"}
              </div>
              {hero.failureClass ? (
                <span className="rounded-full border border-base-300/72 bg-base-100/86 px-2.5 py-1 font-mono text-[11px] text-base-content/58">
                  {hero.failureClass}
                </span>
              ) : null}
            </div>
            <div className="mt-3">
              <SummaryRows rows={summaryRows} />
            </div>
          </div>
        </div>
      </section>

      <section className="rounded-[1.15rem] border border-base-300/72 bg-base-100/60 px-4 py-4 shadow-[0_14px_30px_rgba(15,23,42,0.04)] sm:px-5 sm:py-5">
        <div className="flex flex-wrap items-end justify-between gap-3">
          <div>
            <h3 className="text-sm font-semibold text-base-content">
              {isZh ? "工作流时间线" : "Workflow Timeline"}
            </h3>
          </div>
          <div className="text-xs text-base-content/58">
            {isZh
              ? `${timeline.length.toLocaleString(localeTag)} 个时间线块`
              : `${timeline.length.toLocaleString(localeTag)} timeline blocks`}
          </div>
        </div>

        <div className="relative pl-5">
          <div className="absolute left-[0.55rem] top-3 bottom-3 w-px bg-base-300/72" />
          <div className="space-y-3">
            {timeline.map((entry) => {
              const isOpen = openBlockId === entry.blockId;
              const kindMeta = resolveKindMeta(entry.kind, isZh);

              return (
                <div key={entry.blockId} className="relative">
                  <span
                    className={cn(
                      "absolute left-0 top-5 h-[1.05rem] w-[1.05rem] rounded-full border-2 shadow-[0_0_0_4px_rgba(248,250,252,0.9)]",
                      kindMeta.markerClass,
                    )}
                  />
                  <div className="ml-5">
                    <TimelineSummary
                      entry={entry}
                      localeTag={localeTag}
                      isZh={isZh}
                      isOpen={isOpen}
                      activeSection={
                        isOpen ? (entry.attempt ? attemptSection : genericSection) : null
                      }
                      onSelectSection={(section) => {
                        if (entry.attempt) {
                          toggleAttemptSection(entry, section as AttemptSection);
                        } else {
                          toggleGenericSection(entry, section as GenericSection);
                        }
                      }}
                    />
                    {isOpen && entry.attempt && attemptSection ? (
                      <AttemptDetail
                        record={record}
                        entry={entry}
                        localeTag={localeTag}
                        isZh={isZh}
                        activeSection={attemptSection}
                        requestBodyState={requestBodyState}
                        responseBodyState={responseBodyState}
                      />
                    ) : null}
                    {isOpen && !entry.attempt && genericSection ? (
                      <GenericDetail entry={entry} isZh={isZh} activeSection={genericSection} />
                    ) : null}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </section>
    </div>
  );
}
