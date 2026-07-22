import { useEffect, useRef, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { Badge } from "../../components/ui/badge";
import { Spinner } from "../../components/ui/spinner";
import { Tooltip } from "../../components/ui/tooltip";
import { useTranslation } from "../../i18n";
import type {
  ApiInvocation,
  ApiInvocationRequestBodyResponse,
  ApiInvocationResponseBodyResponse,
  ApiInvocationWorkflowDetailResponse,
  ApiInvocationWorkflowTimelineEntry,
  InvocationCostAudit,
  InvocationCostAuditBreakdown,
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
import {
  renderInvocationCostAuditWarning,
  resolveInvocationCostAuditDisplay,
} from "./invocation-cost-audit";
import { StructuredPayloadViewer } from "./StructuredPayloadViewer";

type DetailPanelSize = "compact" | "default";
export type AttemptSection =
  | "timing"
  | "requestParsed"
  | "requestHeaders"
  | "requestBody"
  | "responseParsed"
  | "responseHeaders"
  | "responseBody";
type GenericSection = "request" | "requestHeaders" | "requestBody" | "json" | "body";

interface PayloadFetchState<T> {
  status: "idle" | "loading" | "loaded" | "error";
  data: T | null;
  error: string | null;
}

interface AttemptUsageAudit {
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

interface TimelineFact {
  key: string;
  label: string;
  tooltip?: string;
}

interface InvocationWorkflowDetailPanelProps {
  record: ApiInvocation;
  focusedAttemptId?: string | null;
  size?: DetailPanelSize;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  hideNonShortIds?: boolean;
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
        variant: "default" as const,
        markerClass: "border-primary/60 bg-primary/14 tone-ink-primary",
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

function readCostAuditBreakdown(value: unknown): InvocationCostAuditBreakdown | null {
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

function readCostAudit(value: unknown): InvocationCostAudit | null {
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

function readAttemptUsageAudit(value: unknown): AttemptUsageAudit | null {
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

function formatPayloadUnavailableReason(reason: string | null | undefined, isZh: boolean) {
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
  return isZh ? "载荷当前不可用。" : "The payload is currently unavailable.";
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

function formatCompactByteSize(value: number | null | undefined, locale: string) {
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

function formatSignedPercent(value: number | null | undefined, locale: string) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  const rounded = Math.round(value);
  if (rounded > 0) return `+${rounded.toLocaleString(locale)}%`;
  return `${rounded.toLocaleString(locale)}%`;
}

function formatRequestCompressionSummary(
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
  const facts: TimelineFact[] = [];
  if (entry.attempt) {
    const attempt = entry.attempt;
    const responseSummary = readRecord(attempt.responseSummary);
    const usageAudit = readAttemptUsageAudit(responseSummary?.usage);
    const phase = formatOptionalText(attempt.phase);
    const upstreamStatus = formatHttpStatus(attempt.httpStatus, localeTag);
    const latencyValue =
      typeof attempt.streamLatencyMs === "number"
        ? `${isZh ? "流式" : "Stream"} ${formatDurationMs(attempt.streamLatencyMs, localeTag)}`
        : typeof attempt.firstByteLatencyMs === "number"
          ? `TTFB ${formatDurationMs(attempt.firstByteLatencyMs, localeTag)}`
          : null;

    if (attempt.upstreamAccountName?.trim()) {
      facts.push({
        key: "upstream-account",
        label: attempt.upstreamAccountName.trim(),
      });
    }
    if (phase !== FALLBACK_CELL) facts.push({ key: "phase", label: phase });
    if (upstreamStatus) {
      facts.push({
        key: "upstream-status",
        label: isZh ? `上游 ${upstreamStatus}` : `Upstream ${upstreamStatus}`,
      });
    }
    if (latencyValue) facts.push({ key: "latency", label: latencyValue });
    if (usageAudit?.cacheWriteTokens != null) {
      facts.push({
        key: "cache-write",
        label: isZh
          ? `输入写 ${usageAudit.cacheWriteTokens.toLocaleString(localeTag)}`
          : `Input write ${usageAudit.cacheWriteTokens.toLocaleString(localeTag)}`,
        tooltip: isZh ? "输入（未命中缓存）" : "Input (uncached)",
      });
    }
    if (usageAudit?.cacheInputTokens != null) {
      facts.push({
        key: "cache-read",
        label: isZh
          ? `输入读 ${usageAudit.cacheInputTokens.toLocaleString(localeTag)}`
          : `Input read ${usageAudit.cacheInputTokens.toLocaleString(localeTag)}`,
        tooltip: isZh ? "输入（命中缓存）" : "Input (cached)",
      });
    }
    if (usageAudit?.outputTokens != null) {
      facts.push({
        key: "output",
        label: isZh
          ? `输出 ${usageAudit.outputTokens.toLocaleString(localeTag)}`
          : `Output ${usageAudit.outputTokens.toLocaleString(localeTag)}`,
      });
    }
    const usageCostDisplay = resolveInvocationCostAuditDisplay(
      usageAudit?.audit,
      usageAudit?.recordedCosts?.total ?? null,
    );
    if (usageCostDisplay.recordedTotal != null) {
      facts.push({
        key: "amount",
        label: isZh
          ? `金额 ${formatCurrency(usageCostDisplay.recordedTotal, localeTag)}`
          : `Amount ${formatCurrency(usageCostDisplay.recordedTotal, localeTag)}`,
      });
    }
    if (attempt.synthetic) facts.push({ key: "synthetic", label: isZh ? "合成尝试" : "Synthetic" });
    return facts;
  }

  if (entry.subtitle?.trim()) facts.push({ key: "subtitle", label: entry.subtitle.trim() });

  const routeRequest = readRecord(entry.detail?.request);
  const routeMode = formatRouteMode(
    readString(routeRequest?.routeMode) ?? readString(entry.detail?.routeMode),
    isZh,
  );
  if (routeMode !== FALLBACK_CELL) facts.push({ key: "route-mode", label: routeMode });

  const poolAttemptCount =
    readNumber(routeRequest?.poolAttemptCount) ?? readNumber(entry.detail?.poolAttemptCount);
  if (poolAttemptCount != null) {
    facts.push({
      key: "pool-attempt-count",
      label: isZh ? `尝试预算 ${poolAttemptCount}` : `Attempt budget ${poolAttemptCount}`,
    });
  }

  const downstreamStatusCode = readNumber(entry.detail?.downstreamStatusCode);
  if (downstreamStatusCode != null) {
    facts.push({
      key: "downstream-status",
      label: `HTTP ${downstreamStatusCode.toLocaleString(localeTag)}`,
    });
  }

  const failureClass = readString(entry.detail?.failureClass);
  if (failureClass) facts.push({ key: "failure-class", label: failureClass });

  return facts;
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
  const requestCompressionSummary = formatRequestCompressionSummary(requestCompression, localeTag);
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
      secondary:
        requestCompressionSummary !== FALLBACK_CELL ? requestCompressionSummary : requestCompaction,
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
  const routeRequest = readRecord(entry.detail?.request);
  const routeRequestHeaders =
    readRecord(entry.detail?.requestHeaders) ?? readRecord(routeRequest?.headers);
  const routeRequestBody =
    readRecord(entry.detail?.requestBody) ?? readRecord(routeRequest?.bodyCapture);
  if (entry.kind === "routingDecision" && routeRequest) {
    const requestModel = formatOptionalText(readString(routeRequest.requestModel));
    const requestTier = formatOptionalText(readString(routeRequest.requestedServiceTier));
    const requestReasoning = formatOptionalText(readString(routeRequest.reasoningEffort));
    const requestTransport = formatOptionalText(readString(routeRequest.transport));
    const requestEndpoint = formatOptionalText(readString(routeRequest.endpoint));
    const requesterIp = formatOptionalText(readString(routeRequest.requesterIp));
    const requestUserAgent = formatOptionalText(readString(routeRequestHeaders?.userAgent));
    const requestBodySize = formatByteSize(readNumber(routeRequestBody?.size), localeTag);
    const requestBodyDetail = formatOptionalText(readString(routeRequestBody?.detailLevel));
    const requestCompaction = formatCompactionSummary(
      readString(routeRequest.compactionRequestKind),
      isZh,
    );
    const routeActions: Array<TimelineMetricAction<GenericSection>> = [
      {
        section: "request",
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
        tertiary: formatOptionalText(
          readString(readRecord(routeRequest?.routing)?.proxyDisplayName),
        ),
      },
      {
        section: "requestBody",
        label: isZh ? "请求体" : "Body",
        primary: compactJoin([requestBodySize, requestBodyDetail]),
        secondary: requestCompaction,
        monospace: true,
      },
    ];
    return routeActions.filter((item) => item.primary !== FALLBACK_CELL);
  }

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
            : formatPayloadUnavailableReason(entry.responseBody.unavailableReason, isZh),
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
  compact = false,
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
  compact?: boolean;
}) {
  const toneClassFor = (variant?: "default" | "secondary" | "success" | "warning" | "error") => {
    if (variant === "success") return "tone-ink-success";
    if (variant === "warning") return "tone-ink-warning";
    if (variant === "error") return "tone-ink-error";
    if (variant === "default") return "tone-ink-info";
    return "text-base-content/88";
  };

  return (
    <dl className="divide-y divide-base-300/42">
      {rows.map((row) => (
        <div
          key={row.label}
          className={cn("flex items-start justify-between gap-4 py-3", compact && "gap-3 py-2.5")}
        >
          <dt
            className={cn("text-[11px] font-medium text-base-content/58", compact && "text-[10px]")}
          >
            {row.label}
          </dt>
          <dd
            className={cn(
              "min-w-0 text-right text-sm font-medium",
              compact && "text-[13px] leading-5",
              toneClassFor(row.variant),
            )}
          >
            {row.action ? (
              <button
                type="button"
                title={row.action.title}
                className={cn(
                  "break-all text-right underline decoration-dotted decoration-current/50 underline-offset-2 transition-colors",
                  row.variant ? toneClassFor(row.variant) : "tone-ink-info",
                  "hover:text-primary focus-visible:outline-none focus-visible:text-primary",
                )}
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
  compact = false,
}: {
  label: string;
  value: string;
  variant?: "default" | "secondary" | "success" | "warning" | "error";
  compact?: boolean;
}) {
  return (
    <div
      className={cn(
        "invocation-detail-subsurface rounded-[0.95rem] px-2.5 py-2.5 sm:px-3 sm:py-3",
        compact && "rounded-[0.8rem] px-2 py-2 sm:px-2.5 sm:py-2.5",
      )}
    >
      <div
        className={cn(
          "text-[11px] font-medium text-base-content/56",
          compact && "text-[10px] leading-4",
        )}
      >
        {label}
      </div>
      <div
        className={cn(
          "mt-1 break-all text-sm font-semibold text-base-content",
          compact && "text-[13px] leading-[1.15]",
          variant === "success" && "tone-ink-success",
          variant === "warning" && "tone-ink-warning",
          variant === "error" && "tone-ink-error",
          variant === "default" && "tone-ink-info",
        )}
      >
        {value}
      </div>
    </div>
  );
}

function OverviewGrid({
  items,
  className,
}: {
  items: Array<{ label: string; value: string; monospace?: boolean }>;
  className?: string;
}) {
  return (
    <dl className={cn("grid gap-x-5 gap-y-4 md:grid-cols-2 xl:grid-cols-3", className)}>
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
  overviewClassName,
}: {
  title: string;
  items: Array<{ label: string; value: string; monospace?: boolean }>;
  overviewClassName?: string;
}) {
  if (items.length === 0) return null;
  return (
    <section className="invocation-detail-subsurface rounded-[0.95rem] px-3.5 py-3">
      <div className="text-[11px] font-medium text-base-content/56">{title}</div>
      <div className="mt-3">
        <OverviewGrid className={overviewClassName} items={items} />
      </div>
    </section>
  );
}

function AttemptUsageAuditPanel({
  usageAudit,
  localeTag,
  isZh,
}: {
  usageAudit: AttemptUsageAudit | null;
  localeTag: string;
  isZh: boolean;
}) {
  const { t } = useTranslation();
  if (!usageAudit) return null;

  const totalCostDisplay = resolveInvocationCostAuditDisplay(
    usageAudit.audit,
    usageAudit.recordedCosts?.total ?? null,
  );
  const metricItems = [
    {
      label: isZh ? "未命中缓存输入 Token" : "Uncached input tokens",
      value: formatOptionalNumber(usageAudit.cacheWriteTokens, localeTag),
    },
    {
      label: isZh ? "命中缓存输入 Token" : "Cached input tokens",
      value: formatOptionalNumber(usageAudit.cacheInputTokens, localeTag),
    },
    {
      label: isZh ? "输出 Token" : "Output tokens",
      value: formatOptionalNumber(usageAudit.outputTokens, localeTag),
    },
    {
      label: isZh ? "金额" : "Amount",
      value: formatCurrency(totalCostDisplay.recordedTotal, localeTag),
    },
  ].filter((item) => item.value !== FALLBACK_CELL);

  const rows = [
    {
      key: "cacheWrite",
      label: isZh ? "未命中缓存输入成本" : "Uncached input cost",
      recorded: usageAudit.recordedCosts?.cacheWrite ?? null,
      local: usageAudit.localCosts?.cacheWrite ?? null,
    },
    {
      key: "cacheRead",
      label: isZh ? "命中缓存输入成本" : "Cached input cost",
      recorded: usageAudit.recordedCosts?.cacheRead ?? null,
      local: usageAudit.localCosts?.cacheRead ?? null,
    },
    {
      key: "output",
      label: isZh ? "输出成本" : "Output cost",
      recorded: usageAudit.recordedCosts?.output ?? null,
      local: usageAudit.localCosts?.output ?? null,
    },
    {
      key: "reasoning",
      label: isZh ? "推理成本" : "Reasoning cost",
      recorded: usageAudit.recordedCosts?.reasoning ?? null,
      local: usageAudit.localCosts?.reasoning ?? null,
    },
    {
      key: "total",
      label: isZh ? "总成本" : "Total cost",
      recorded: totalCostDisplay.recordedTotal,
      local: totalCostDisplay.localTotal,
    },
  ];

  return (
    <>
      <DetailMetaStrip items={metricItems} />
      <section className="invocation-detail-subsurface rounded-[0.95rem] px-3.5 py-3">
        <div className="flex items-center justify-between gap-3">
          <div className="text-[11px] font-medium text-base-content/56">
            {isZh ? "Token 与成本" : "Token and cost"}
          </div>
          {renderInvocationCostAuditWarning(
            usageAudit.audit,
            t,
            (value) => formatCurrency(value, localeTag),
            { testId: "workflow-usage-cost-warning" },
          )}
        </div>
        <div className="mt-3 space-y-2">
          {rows.map((row) => {
            return (
              <div
                key={row.key}
                className="grid grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-3 rounded-xl border border-base-300/60 bg-base-100/72 px-3 py-2 text-xs"
              >
                <div className="min-w-0 text-base-content/68">{row.label}</div>
                <div className="font-mono text-base-content/84">
                  {formatCurrency(row.recorded, localeTag)}
                </div>
                <div className="font-mono text-base-content/62">
                  {`${isZh ? "本地" : "Local"} ${formatCurrency(row.local, localeTag)}`}
                </div>
              </div>
            );
          })}
          {!totalCostDisplay.mismatch && totalCostDisplay.reason ? (
            <div className="rounded-xl border border-base-300/55 bg-base-100/65 px-3 py-2 text-xs text-base-content/58">
              {t("records.costAudit.notComparable")}
            </div>
          ) : null}
        </div>
      </section>
    </>
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
    <section className="invocation-detail-subsurface flex flex-wrap gap-x-4 gap-y-2 rounded-[0.95rem] px-3 py-2.5">
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
    <div className="invocation-detail-subsurface min-w-0 max-w-full overflow-hidden space-y-2.5 rounded-b-[1rem] border-t-0 px-3.5 pb-3.5 pt-2.5">
      {controls ? (
        <div className="flex min-w-0 items-center justify-between gap-3">{controls}</div>
      ) : null}
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
        tone === "default" && "invocation-detail-subsurface text-base-content/64",
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
  hideNonShortIds = false,
}: {
  record: ApiInvocation;
  entry: ApiInvocationWorkflowTimelineEntry;
  localeTag: string;
  isZh: boolean;
  activeSection: AttemptSection;
  requestBodyState: PayloadFetchState<ApiInvocationRequestBodyResponse>;
  responseBodyState: PayloadFetchState<ApiInvocationResponseBodyResponse>;
  hideNonShortIds?: boolean;
}) {
  const attempt = entry.attempt;
  if (!attempt) return null;
  const labels = buildPayloadViewerLabels(isZh);
  const requestSummary = readRecord(attempt.requestSummary);
  const responseSummary = readRecord(attempt.responseSummary);
  const usageAudit = readAttemptUsageAudit(responseSummary?.usage);
  const requestBodyParsed = requestBodyState.data?.bodyText
    ? extractRequestBusinessSnapshot(requestBodyState.data.bodyText)
    : null;
  const responseBodyParsed = responseBodyState.data?.bodyText
    ? extractResponseBusinessSnapshot(responseBodyState.data.bodyText)
    : null;
  const requestHeaderSource =
    readRecord(requestSummary?.headers) ?? readRecord(requestBodyState.data?.headers);
  const requestCompression = readRecord(requestSummary?.compression);
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
  const responseBodyCaptureSource = readRecord(responseSummary?.responseBodyCapture);
  const requestArchiveAtInvocation = readBoolean(
    requestBodyCaptureSource?.availableAtInvocationLevel,
  );
  const responseArchiveAtInvocation = readBoolean(
    responseBodyCaptureSource?.availableAtInvocationLevel,
  );
  const responseBodyUnavailableReason =
    responseBodyState.data?.unavailableReason ??
    readString(responseBodyCaptureSource?.unavailableReason);

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
    ...(!hideNonShortIds
      ? [
          {
            label: isZh ? "上游请求 ID" : "Upstream Request ID",
            value: formatOptionalText(attempt.upstreamRequestId),
          },
        ]
      : []),
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
  const requestCompressionSummary = formatRequestCompressionSummary(requestCompression, localeTag);

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
  const requestCompressionItems = [
    {
      label: isZh ? "压缩比" : "Compression ratio",
      value: requestCompressionSummary,
      monospace: false,
    },
    {
      label: isZh ? "算法" : "Algorithm",
      value: formatOptionalText(readString(requestCompression?.algorithm)),
      monospace: false,
    },
    {
      label: isZh ? "发送模式" : "Mode",
      value: formatOptionalText(readString(requestCompression?.mode)),
      monospace: false,
    },
    {
      label: isZh ? "近似上传" : "Approx upload",
      value: formatCompactByteSize(readNumber(requestCompression?.approxUploadBytes), localeTag),
      monospace: false,
    },
    {
      label: isZh ? "近似下载" : "Approx download",
      value: formatCompactByteSize(readNumber(requestCompression?.approxDownloadBytes), localeTag),
      monospace: false,
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
    ...(!hideNonShortIds
      ? [{ key: "upstreamRequestId", label: "X-Request-ID", monospace: false }]
      : []),
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
      value:
        responseArchiveAtInvocation == null || responseArchiveAtInvocation
          ? isZh
            ? "调用级"
            : "Invocation"
          : isZh
            ? "尝试指标"
            : "Attempt metrics",
      monospace: false,
    },
    {
      label: isZh ? "来源" : "Source",
      value: formatOptionalText(responseBodyState.data?.captureSource),
      monospace: false,
    },
    {
      label: isZh ? "大小" : "Size",
      value: formatByteSize(
        responseBodyState.data?.bodySize ?? readNumber(responseBodyCaptureSource?.size),
        localeTag,
      ),
      monospace: false,
    },
    {
      label: isZh ? "详情" : "Detail",
      value: formatOptionalText(
        responseBodyState.data?.detailLevel ?? readString(responseBodyCaptureSource?.detailLevel),
      ),
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
          <DetailInfoPanel
            title={isZh ? "HTTP 请求压缩" : "HTTP request compression"}
            items={requestCompressionItems}
            overviewClassName="lg:grid-cols-5 xl:grid-cols-5"
          />
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
              {formatPayloadUnavailableReason(requestBodyState.data?.unavailableReason, isZh)}
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
          <AttemptUsageAuditPanel usageAudit={usageAudit} localeTag={localeTag} isZh={isZh} />
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
              {formatPayloadUnavailableReason(responseBodyUnavailableReason, isZh)}
            </PayloadNotice>
          )}
        </>
      ) : null}
    </DetailFrame>
  );
}

function GenericDetail({
  entry,
  localeTag,
  isZh,
  activeSection,
  requestBodyState,
}: {
  entry: ApiInvocationWorkflowTimelineEntry;
  localeTag: string;
  isZh: boolean;
  activeSection: GenericSection;
  requestBodyState: PayloadFetchState<ApiInvocationRequestBodyResponse>;
}) {
  const labels = buildPayloadViewerLabels(isZh);
  const detailContent = stringifyStructuredValue(entry.detail ?? undefined);
  const bodyText = entry.responseBody?.bodyText?.trim() ?? "";
  const routeRequest = readRecord(entry.detail?.request);
  const routeRequestHeaders =
    readRecord(entry.detail?.requestHeaders) ?? readRecord(routeRequest?.headers);
  const routeRequestRouting = readRecord(routeRequest?.routing);
  const routeRequestClient = readRecord(routeRequest?.client);
  const routeRequestAccount = readRecord(routeRequest?.account);
  const routeRequestCompression = readRecord(routeRequest?.compression);
  const routeRequestBody =
    readRecord(entry.detail?.requestBody) ?? readRecord(routeRequest?.bodyCapture);
  const routeRequestParsedItems = [
    ...buildStructuredItems(routeRequest, localeTag, isZh, [
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
      { key: "promptCacheKey", label: "Prompt Cache Key" },
      { key: "stickyKey", label: "Sticky Key" },
      { key: "requesterIp", label: isZh ? "请求 IP" : "Requester IP", monospace: false },
    ]),
    ...buildStructuredItems(routeRequestAccount, localeTag, isZh, [
      { key: "id", label: isZh ? "账号 ID" : "Account ID", monospace: false },
      { key: "name", label: isZh ? "账号" : "Account", monospace: false },
    ]),
    ...buildStructuredItems(routeRequestCompression, localeTag, isZh, [
      { key: "algorithm", label: isZh ? "压缩算法" : "Compression", monospace: false },
      { key: "mode", label: isZh ? "压缩模式" : "Compression Mode", monospace: false },
    ]),
  ];
  const routeRequestRoutingItems = [
    ...buildStructuredItems(routeRequestRouting, localeTag, isZh, [
      { key: "routeMode", label: isZh ? "路由模式" : "Route Mode", monospace: false },
      { key: "upstreamScope", label: isZh ? "上游范围" : "Upstream Scope", monospace: false },
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
    ...buildStructuredItems(routeRequestClient, localeTag, isZh, [
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
  const routeHeaderItems = buildStructuredItems(routeRequestHeaders, localeTag, isZh, [
    { key: "userAgent", label: "User-Agent", monospace: false },
    { key: "xForwardedFor", label: "X-Forwarded-For", monospace: false },
    { key: "forwarded", label: "Forwarded", monospace: false },
    { key: "xRealIp", label: "X-Real-IP", monospace: false },
  ]);
  const routeBodySummaryItems = [
    {
      label: isZh ? "归档" : "Archive",
      value:
        readBoolean(routeRequestBody?.availableAtInvocationLevel) == null
          ? FALLBACK_CELL
          : readBoolean(routeRequestBody?.availableAtInvocationLevel)
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
      value: formatByteSize(
        requestBodyState.data?.bodySize ?? readNumber(routeRequestBody?.size),
        localeTag,
      ),
      monospace: false,
    },
    {
      label: isZh ? "详情" : "Detail",
      value: formatOptionalText(
        requestBodyState.data?.detailLevel ?? readString(routeRequestBody?.detailLevel),
      ),
      monospace: false,
    },
    {
      label: isZh ? "截断" : "Truncated",
      value:
        requestBodyState.data?.bodyTruncated == null &&
        readBoolean(routeRequestBody?.truncated) == null
          ? FALLBACK_CELL
          : (requestBodyState.data?.bodyTruncated ?? readBoolean(routeRequestBody?.truncated))
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
      value: formatOptionalText(
        requestBodyState.data?.bodyTruncatedReason ?? readString(routeRequestBody?.truncatedReason),
      ),
      monospace: false,
      fullWidth: true,
    },
    {
      label: isZh ? "裁剪原因" : "Prune Reason",
      value: formatOptionalText(
        requestBodyState.data?.detailPruneReason ?? readString(routeRequestBody?.detailPruneReason),
      ),
      monospace: false,
      fullWidth: true,
    },
  ].filter((item) => item.value !== FALLBACK_CELL);
  const routeRequestBodyContent = requestBodyState.data?.bodyText?.trim() ?? "";
  return (
    <DetailFrame>
      {entry.kind === "routingDecision" && activeSection === "request" ? (
        <>
          <DetailInfoPanel
            title={isZh ? "解析后的请求" : "Parsed request"}
            items={routeRequestParsedItems}
          />
          <DetailInfoPanel
            title={isZh ? "路由与会话信号" : "Routing and session"}
            items={routeRequestRoutingItems}
          />
          <DetailMetaStrip items={routeBodySummaryItems} />
        </>
      ) : null}

      {entry.kind === "routingDecision" && activeSection === "requestHeaders" ? (
        <>
          <DetailInfoPanel title={isZh ? "请求头" : "Request headers"} items={routeHeaderItems} />
          <DetailInfoPanel
            title={isZh ? "路由与会话信号" : "Routing and session"}
            items={routeRequestRoutingItems}
          />
        </>
      ) : null}

      {entry.kind === "routingDecision" && activeSection === "requestBody" ? (
        <>
          <DetailMetaStrip items={routeBodySummaryItems} />
          {requestBodyState.status === "loading" ? (
            <PayloadNotice>{isZh ? "加载请求体…" : "Loading request body…"}</PayloadNotice>
          ) : requestBodyState.status === "error" ? (
            <PayloadNotice tone="error">
              {isZh ? "请求体加载失败：" : "Failed to load request body: "}
              {requestBodyState.error}
            </PayloadNotice>
          ) : requestBodyState.data?.available && routeRequestBodyContent ? (
            <StructuredPayloadViewer value={routeRequestBodyContent} labels={labels} />
          ) : (
            <PayloadNotice tone="warning">
              {isZh ? "请求体不可用：" : "Request body unavailable: "}
              {formatPayloadUnavailableReason(requestBodyState.data?.unavailableReason, isZh)}
            </PayloadNotice>
          )}
        </>
      ) : null}

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
            {formatPayloadUnavailableReason(entry.responseBody.unavailableReason, isZh)}
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
  attemptIdentityOverride,
  testId,
}: {
  entry: ApiInvocationWorkflowTimelineEntry;
  localeTag: string;
  isZh: boolean;
  isOpen: boolean;
  activeSection: AttemptSection | GenericSection | null;
  onSelectSection: (section: AttemptSection | GenericSection) => void;
  attemptIdentityOverride?: string | null;
  testId?: string;
}) {
  const kindMeta = resolveKindMeta(entry.kind, isZh);
  const statusMeta = resolveStatusMeta(entry.status, isZh);
  const summaryFacts = buildTimelineFacts(entry, isZh, localeTag);
  const attemptId = attemptIdentityOverride?.trim() || entry.attempt?.attemptId?.trim() || null;
  const showTitle = !entry.attempt;
  const metricActions = entry.attempt
    ? buildAttemptMetricActions(entry, localeTag, isZh)
    : buildGenericMetricActions(entry, localeTag, isZh);

  return (
    <div
      data-testid={testId}
      data-open={isOpen ? "true" : "false"}
      className={cn(
        "invocation-detail-block w-full min-w-0 max-w-full overflow-hidden rounded-[1rem] px-4 py-3 text-left transition-[background-color,border-color] duration-200",
        !isOpen && "hover:border-[var(--invocation-detail-subsurface-border-active)]",
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
                {summaryFacts.map((fact) =>
                  fact.tooltip ? (
                    <Tooltip
                      key={`${entry.blockId}-${fact.key}`}
                      content={fact.tooltip}
                      side="top"
                      sideOffset={8}
                    >
                      <span className="invocation-detail-fact-chip min-w-0 break-all rounded-full px-2 py-0.5">
                        {fact.label}
                      </span>
                    </Tooltip>
                  ) : (
                    <span
                      key={`${entry.blockId}-${fact.key}`}
                      className="invocation-detail-fact-chip min-w-0 break-all rounded-full px-2 py-0.5"
                    >
                      {fact.label}
                    </span>
                  ),
                )}
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
        <div className="invocation-detail-rail mt-3 overflow-hidden rounded-[0.95rem]">
          <div className="grid gap-px sm:grid-cols-2 lg:grid-cols-4 xl:grid-cols-7">
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

interface InvocationWorkflowAttemptRecordProps {
  record: ApiInvocation;
  entry: ApiInvocationWorkflowTimelineEntry;
  localeTag: string;
  isZh: boolean;
  summaryIdentity?: string | null;
  focused?: boolean;
  focusVersion?: number;
  defaultSection?: AttemptSection | null;
  isOpen?: boolean;
  activeSection?: AttemptSection | null;
  onSelectSection?: (section: AttemptSection) => void;
  hideNonShortIds?: boolean;
  className?: string;
  containerRef?: (node: HTMLDivElement | null) => void;
  testId?: string;
}

export function InvocationWorkflowAttemptRecord({
  record,
  entry,
  localeTag,
  isZh,
  summaryIdentity,
  focused = false,
  focusVersion = 0,
  defaultSection = null,
  isOpen,
  activeSection,
  onSelectSection,
  hideNonShortIds = false,
  className,
  containerRef,
  testId,
}: InvocationWorkflowAttemptRecordProps) {
  const isControlled = isOpen !== undefined && activeSection !== undefined && !!onSelectSection;
  const requestBodyFetchSeqRef = useRef(0);
  const responseBodyFetchSeqRef = useRef(0);
  const [internalSection, setInternalSection] = useState<AttemptSection | null>(defaultSection);
  const [requestBodyState, setRequestBodyState] = useState<
    PayloadFetchState<ApiInvocationRequestBodyResponse>
  >(createIdlePayloadState());
  const [responseBodyState, setResponseBodyState] = useState<
    PayloadFetchState<ApiInvocationResponseBodyResponse>
  >(createIdlePayloadState());

  const currentSection = isControlled ? activeSection : internalSection;
  const currentOpen = isControlled ? isOpen : currentSection != null;
  const responseBodyCapture = readRecord(entry.attempt?.responseSummary?.responseBodyCapture);
  const responseBodyAvailableAtInvocationLevel = readBoolean(
    responseBodyCapture?.availableAtInvocationLevel,
  );

  useEffect(() => {
    requestBodyFetchSeqRef.current += 1;
    responseBodyFetchSeqRef.current += 1;
    setRequestBodyState(createIdlePayloadState());
    setResponseBodyState(createIdlePayloadState());
  }, [entry.blockId, record.id]);

  useEffect(() => {
    if (isControlled || !focused || !defaultSection) return;
    setInternalSection(defaultSection);
  }, [defaultSection, focusVersion, focused, isControlled]);

  useEffect(() => {
    if (!(record.id > 0)) return;
    if (
      !currentSection ||
      !isRequestSection(currentSection) ||
      requestBodyState.status !== "idle"
    ) {
      return;
    }

    const requestSeq = requestBodyFetchSeqRef.current + 1;
    requestBodyFetchSeqRef.current = requestSeq;
    setRequestBodyState({ status: "loading", data: null, error: null });
    void fetchInvocationRequestBody(record.id)
      .then((data) => {
        if (requestSeq !== requestBodyFetchSeqRef.current) return;
        setRequestBodyState({ status: "loaded", data, error: null });
      })
      .catch((error) => {
        if (requestSeq !== requestBodyFetchSeqRef.current) return;
        setRequestBodyState({
          status: "error",
          data: null,
          error: error instanceof Error ? error.message : String(error),
        });
      });
  }, [currentSection, record.id, requestBodyState.status]);

  useEffect(() => {
    if (!(record.id > 0)) return;
    if (responseBodyAvailableAtInvocationLevel === false) return;
    if (
      !currentSection ||
      !isResponseSection(currentSection) ||
      responseBodyState.status !== "idle"
    ) {
      return;
    }

    const requestSeq = responseBodyFetchSeqRef.current + 1;
    responseBodyFetchSeqRef.current = requestSeq;
    setResponseBodyState({ status: "loading", data: null, error: null });
    void fetchInvocationResponseBody(record.id)
      .then((data) => {
        if (requestSeq !== responseBodyFetchSeqRef.current) return;
        setResponseBodyState({ status: "loaded", data, error: null });
      })
      .catch((error) => {
        if (requestSeq !== responseBodyFetchSeqRef.current) return;
        setResponseBodyState({
          status: "error",
          data: null,
          error: error instanceof Error ? error.message : String(error),
        });
      });
  }, [currentSection, record.id, responseBodyAvailableAtInvocationLevel, responseBodyState.status]);

  const handleSelectSection = (section: AttemptSection) => {
    if (isControlled) {
      onSelectSection?.(section);
      return;
    }
    setInternalSection((current) => (current === section ? null : section));
  };

  if (!entry.attempt) return null;

  return (
    <div
      ref={containerRef}
      className={cn(
        "min-w-0 max-w-full scroll-mt-4 rounded-[1.125rem] border border-transparent transition-[background-color,border-color,box-shadow] duration-200",
        focused && "border-primary/45 bg-primary/8 ring-1 ring-inset ring-primary/35",
        className,
      )}
      data-focus-visible={focused ? "true" : "false"}
      data-testid={testId}
      aria-current={focused ? "true" : undefined}
    >
      <TimelineSummary
        entry={entry}
        localeTag={localeTag}
        isZh={isZh}
        isOpen={currentOpen}
        activeSection={currentSection}
        onSelectSection={(section) => handleSelectSection(section as AttemptSection)}
        attemptIdentityOverride={summaryIdentity}
      />
      {currentOpen && currentSection ? (
        <AttemptDetail
          record={record}
          entry={entry}
          localeTag={localeTag}
          isZh={isZh}
          activeSection={currentSection}
          requestBodyState={requestBodyState}
          responseBodyState={responseBodyState}
          hideNonShortIds={hideNonShortIds}
        />
      ) : null}
    </div>
  );
}

export function InvocationWorkflowDetailPanel({
  record,
  focusedAttemptId = null,
  size = "default",
  onOpenUpstreamAccount,
  hideNonShortIds = false,
}: InvocationWorkflowDetailPanelProps) {
  const { locale } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const isZh = locale === "zh";
  const requestSeqRef = useRef(0);
  const requestBodyFetchSeqRef = useRef(0);
  const [detail, setDetail] = useState<ApiInvocationWorkflowDetailResponse | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [openBlockId, setOpenBlockId] = useState<string | null>(null);
  const [attemptSection, setAttemptSection] = useState<AttemptSection | null>(null);
  const [genericSection, setGenericSection] = useState<GenericSection | null>(null);
  const [requestBodyState, setRequestBodyState] = useState<
    PayloadFetchState<ApiInvocationRequestBodyResponse>
  >(createIdlePayloadState());

  useEffect(() => {
    if (!(record.id > 0)) {
      requestSeqRef.current += 1;
      requestBodyFetchSeqRef.current += 1;
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
    requestBodyFetchSeqRef.current += 1;
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
  }, [detail]);

  useEffect(() => {
    if (!detail || !focusedAttemptId) return;
    const focusedEntry = detail.timeline.find(
      (entry) => entry.attempt?.attemptId === focusedAttemptId,
    );
    if (!focusedEntry?.attempt) return;
    setOpenBlockId(focusedEntry.blockId);
    setAttemptSection("timing");
    setGenericSection(null);
  }, [detail, focusedAttemptId]);

  useEffect(() => {
    if (!(record.id > 0)) return;
    if (genericSection !== "requestBody" || requestBodyState.status !== "idle") return;

    const requestSeq = requestBodyFetchSeqRef.current + 1;
    requestBodyFetchSeqRef.current = requestSeq;
    setRequestBodyState({ status: "loading", data: null, error: null });
    void fetchInvocationRequestBody(record.id)
      .then((data) => {
        if (requestSeq !== requestBodyFetchSeqRef.current) return;
        setRequestBodyState({ status: "loaded", data, error: null });
      })
      .catch((error) => {
        if (requestSeq !== requestBodyFetchSeqRef.current) return;
        setRequestBodyState({
          status: "error",
          data: null,
          error: error instanceof Error ? error.message : String(error),
        });
      });
  }, [genericSection, record.id, requestBodyState.status]);

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
  const modelTrailCounts = new Map<string, number>();
  const modelTrailItems = modelTrail.map((value) => {
    const occurrence = modelTrailCounts.get(value) ?? 0;
    modelTrailCounts.set(value, occurrence + 1);
    return {
      key: `${value}-${occurrence}`,
      value,
    };
  });

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
  const isCompact = size === "compact";

  return (
    <div
      className={cn(
        "min-w-0 max-w-full overflow-hidden space-y-4",
        isCompact ? "invocation-detail-mobile-flat text-sm" : "",
      )}
    >
      <section
        className={cn(
          "invocation-detail-hero-surface min-w-0 max-w-full overflow-hidden rounded-[1.2rem] px-4 py-4 sm:px-5 sm:py-5",
          isCompact && "rounded-none px-0 py-0",
        )}
      >
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
            isCompact
              ? "gap-3 xl:grid-cols-[minmax(0,1.4fr)_minmax(19rem,0.9fr)]"
              : "xl:grid-cols-[minmax(0,1.45fr)_minmax(22rem,0.95fr)]",
          )}
        >
          <div
            className={cn(
              "invocation-detail-card-surface rounded-[1rem] p-4",
              isCompact && "rounded-none p-0",
            )}
          >
            <div
              className={cn(
                "grid gap-4 lg:grid-cols-[minmax(0,1.1fr)_minmax(14rem,0.9fr)]",
                isCompact && "gap-3",
              )}
            >
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

            <div
              className={cn(
                "mt-4 grid grid-cols-2 gap-2.5 sm:grid-cols-4 sm:gap-3",
                isCompact &&
                  (isZh
                    ? "mt-3 grid-cols-4 gap-2 sm:gap-2.5"
                    : "mt-3 gap-2 min-[496px]:grid-cols-4 sm:gap-2.5"),
              )}
            >
              {snapshotMetrics.map((metric) => (
                <SnapshotMetric
                  key={metric.label}
                  label={metric.label}
                  value={metric.value}
                  variant={metric.variant}
                  compact={isCompact}
                />
              ))}
            </div>

            <div
              className={cn(
                "mt-4 grid gap-4 border-t border-base-300/65 pt-4 lg:grid-cols-[minmax(0,1fr)_minmax(16rem,0.95fr)]",
                isCompact && "mt-3 gap-3 pt-3",
              )}
            >
              <div className="min-w-0">
                <div className="text-[11px] font-medium text-base-content/56">
                  {isZh ? "模型与端点" : "Models and Endpoint"}
                </div>
                <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-sm text-base-content/84">
                  {modelTrailItems.length > 0 ? (
                    <>
                      {modelTrailItems.map((modelItem) => (
                        <span key={modelItem.key} className="min-w-0 break-all">
                          {modelItem.value}
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

          <div
            className={cn(
              "invocation-detail-card-surface rounded-[1rem] p-4",
              isCompact && "rounded-none p-0",
            )}
          >
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
            <div className={cn("mt-3", isCompact && "mt-2.5")}>
              <SummaryRows rows={summaryRows} compact={isCompact} />
            </div>
          </div>
        </div>
      </section>

      <section className="invocation-detail-timeline-surface min-w-0 max-w-full overflow-hidden rounded-[1.15rem] px-4 py-4 sm:px-5 sm:py-5">
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

        <div className={cn("relative min-w-0", size === "compact" ? "pl-4" : "pl-5")}>
          <div
            className={cn(
              "absolute top-3 bottom-3 w-px bg-base-300/72",
              size === "compact" ? "left-[0.45rem]" : "left-[0.55rem]",
            )}
          />
          <div className="space-y-3">
            {timeline.map((entry) => {
              const isOpen = openBlockId === entry.blockId;
              const kindMeta = resolveKindMeta(entry.kind, isZh);

              return (
                <div key={entry.blockId} className="relative min-w-0">
                  <span
                    className={cn(
                      "invocation-detail-marker absolute left-0 top-5 h-[1.05rem] w-[1.05rem] rounded-full border-2",
                      kindMeta.markerClass,
                    )}
                  />
                  <div className={cn("min-w-0", size === "compact" ? "ml-4" : "ml-5")}>
                    {entry.attempt ? (
                      <InvocationWorkflowAttemptRecord
                        record={record}
                        entry={entry}
                        localeTag={localeTag}
                        isZh={isZh}
                        isOpen={isOpen}
                        activeSection={isOpen ? attemptSection : null}
                        onSelectSection={(section) => toggleAttemptSection(entry, section)}
                        hideNonShortIds={hideNonShortIds}
                      />
                    ) : (
                      <>
                        <TimelineSummary
                          entry={entry}
                          localeTag={localeTag}
                          isZh={isZh}
                          isOpen={isOpen}
                          activeSection={isOpen ? genericSection : null}
                          onSelectSection={(section) =>
                            toggleGenericSection(entry, section as GenericSection)
                          }
                        />
                        {isOpen && genericSection ? (
                          <GenericDetail
                            entry={entry}
                            localeTag={localeTag}
                            isZh={isZh}
                            activeSection={genericSection}
                            requestBodyState={requestBodyState}
                          />
                        ) : null}
                      </>
                    )}
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
