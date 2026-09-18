import type { ApiInvocationWorkflowTimelineEntry } from "../../lib/api";
import {
  isFiniteNonNegativeMilliseconds,
  isFinitePositiveMilliseconds,
} from "../../lib/invocationTiming";
import { formatReasoningEffort } from "../shared/reasoningEffort";
import type {
  AttemptSection,
  GenericSection,
  TimelineFact,
  TimelineMetricAction,
  Translator,
} from "./InvocationWorkflowDetailPanel.formatters";
import {
  buildToolChips,
  compactJoin,
  FALLBACK_CELL,
  formatByteSize,
  formatCompactionSummary,
  formatCurrency,
  formatDurationMs,
  formatHttpCompressionTag,
  formatHttpStatus,
  formatOptionalText,
  formatPayloadUnavailableReason,
  formatRequestCompressionSummary,
  formatResponseDurationMs,
  formatRouteMode,
  readAttemptUsageAudit,
  readNumber,
  readRecord,
  readString,
  resolveEndpointMetricDisplay,
  stringifyStructuredValue,
  summarizeOutputItems,
  summarizeToolCalls,
} from "./InvocationWorkflowDetailPanel.formatters";
import { resolveInvocationCostAuditDisplay } from "./invocation-cost-audit";

export function buildAttemptTimelineFacts(
  attempt: NonNullable<ApiInvocationWorkflowTimelineEntry["attempt"]>,
  isZh: boolean,
  localeTag: string,
) {
  const facts: TimelineFact[] = [];
  const responseSummary = readRecord(attempt.responseSummary);
  const usageAudit = readAttemptUsageAudit(responseSummary?.usage);
  const phase = formatOptionalText(attempt.phase);
  const upstreamStatus = formatHttpStatus(attempt.httpStatus, localeTag);
  const latencyValue = isFinitePositiveMilliseconds(attempt.streamLatencyMs)
    ? `${isZh ? "流式" : "Stream"} ${formatResponseDurationMs(attempt.streamLatencyMs, localeTag)}`
    : isFiniteNonNegativeMilliseconds(attempt.firstTokenMs)
      ? `TTFT ${formatDurationMs(attempt.firstTokenMs, localeTag)}`
      : null;

  if (attempt.upstreamAccountName?.trim()) {
    facts.push({ key: "upstream-account", label: attempt.upstreamAccountName.trim() });
  }
  if (phase !== FALLBACK_CELL) facts.push({ key: "phase", label: phase });
  if (upstreamStatus) {
    facts.push({
      key: "upstream-status",
      label: isZh ? `上游 ${upstreamStatus}` : `Upstream ${upstreamStatus}`,
    });
  }
  if (latencyValue) {
    facts.push({
      key: "latency",
      label: latencyValue,
      tone: isFinitePositiveMilliseconds(attempt.streamLatencyMs)
        ? "secondary"
        : isFiniteNonNegativeMilliseconds(attempt.firstTokenMs)
          ? "success"
          : "secondary",
    });
  }
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

export function buildGenericTimelineFacts(
  entry: ApiInvocationWorkflowTimelineEntry,
  isZh: boolean,
  localeTag: string,
) {
  const facts: TimelineFact[] = [];
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

export function buildTimelineFacts(
  entry: ApiInvocationWorkflowTimelineEntry,
  isZh: boolean,
  localeTag: string,
) {
  return entry.attempt
    ? buildAttemptTimelineFacts(entry.attempt, isZh, localeTag)
    : buildGenericTimelineFacts(entry, isZh, localeTag);
}

export function buildAttemptMetricContext(
  entry: ApiInvocationWorkflowTimelineEntry,
  localeTag: string,
  isZh: boolean,
  t: Translator,
) {
  const attempt = entry.attempt;
  if (!attempt) return null;
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
  const requestReasoning = formatReasoningEffort(readString(requestSummary?.reasoningEffort));
  const requestTransport = formatOptionalText(readString(requestSummary?.transport));
  const requestEndpoint = resolveEndpointMetricDisplay({
    endpoint: readString(requestSummary?.endpoint),
    status: attempt.status,
    compactionRequestKind: readString(requestSummary?.compactionRequestKind),
    compactionResponseKind: readString(responseSummary?.compactionResponseKind),
    t,
  });
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
  return {
    attempt,
    requestModel,
    responseModel,
    requesterIp,
    responseStatus,
    responseHeadersMetric,
    requestTier,
    requestReasoning,
    requestTransport,
    requestEndpoint,
    requestCompaction,
    proxyDisplay,
    requestUserAgent,
    requestBodySize,
    requestBodyDetail,
    requestCompressionSummary,
    responseFailureKind,
    responseTier,
    responseCompaction,
    responseToolChips,
    responseToolSummary,
    responseBodySize,
    responseBodyDetail,
    responseBodySummary,
    requestHttpCompressionTag,
    responseHttpCompressionTag,
  };
}

export function buildAttemptMetricActions(
  entry: ApiInvocationWorkflowTimelineEntry,
  localeTag: string,
  isZh: boolean,
  t: Translator,
): Array<TimelineMetricAction<AttemptSection>> {
  const context = buildAttemptMetricContext(entry, localeTag, isZh, t);
  if (!context) return [];
  const {
    attempt,
    requestModel,
    responseModel,
    requesterIp,
    responseStatus,
    responseHeadersMetric,
    requestTier,
    requestReasoning,
    requestTransport,
    requestEndpoint,
    requestCompaction,
    proxyDisplay,
    requestUserAgent,
    requestBodySize,
    requestBodyDetail,
    requestCompressionSummary,
    responseFailureKind,
    responseTier,
    responseCompaction,
    responseToolChips,
    responseToolSummary,
    responseBodySize,
    responseBodyDetail,
    responseBodySummary,
    requestHttpCompressionTag,
    responseHttpCompressionTag,
  } = context;

  const actions: Array<TimelineMetricAction<AttemptSection>> = [
    {
      section: "timing",
      label: isZh ? "时间" : "Timing",
      primary: formatResponseDurationMs(attempt.streamLatencyMs, localeTag),
      secondary: `TTFT ${formatDurationMs(attempt.firstTokenMs, localeTag)}`,
      secondaryTone: isFiniteNonNegativeMilliseconds(attempt.firstTokenMs) ? "success" : undefined,
    },
    {
      section: "requestParsed",
      label: isZh ? "请求" : "Request",
      tag: requestEndpoint.tag,
      primary: requestModel,
      secondary: compactJoin([requestTier, requestReasoning]),
      tertiary: compactJoin([requestTransport, requestEndpoint.tertiary]),
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

  return actions.filter(
    (item) => item.primary !== FALLBACK_CELL || item.secondaryTone === "success",
  );
}

export function buildRoutingMetricActions(
  entry: ApiInvocationWorkflowTimelineEntry,
  routeRequest: Record<string, unknown>,
  routeRequestHeaders: Record<string, unknown> | null,
  routeRequestBody: Record<string, unknown> | null,
  localeTag: string,
  isZh: boolean,
  t: Translator,
): Array<TimelineMetricAction<GenericSection>> {
  const requestModel = formatOptionalText(readString(routeRequest.requestModel));
  const requestTier = formatOptionalText(readString(routeRequest.requestedServiceTier));
  const requestReasoning = formatReasoningEffort(readString(routeRequest.reasoningEffort));
  const requestTransport = formatOptionalText(readString(routeRequest.transport));
  const requestEndpoint = resolveEndpointMetricDisplay({
    endpoint: readString(routeRequest.endpoint),
    status: entry.status,
    compactionRequestKind: readString(routeRequest.compactionRequestKind),
    t,
  });
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
      tag: requestEndpoint.tag,
      primary: requestModel,
      secondary: compactJoin([requestTier, requestReasoning]),
      tertiary: compactJoin([requestTransport, requestEndpoint.tertiary]),
    },
    {
      section: "requestHeaders",
      label: isZh ? "请求头" : "Headers",
      primary: requesterIp,
      secondary: requestUserAgent,
      tertiary: formatOptionalText(readString(readRecord(routeRequest.routing)?.proxyDisplayName)),
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

export function buildGenericMetricActions(
  entry: ApiInvocationWorkflowTimelineEntry,
  localeTag: string,
  isZh: boolean,
  t: Translator,
): Array<TimelineMetricAction<GenericSection>> {
  const routeRequest = readRecord(entry.detail?.request);
  const routeRequestHeaders =
    readRecord(entry.detail?.requestHeaders) ?? readRecord(routeRequest?.headers);
  const routeRequestBody =
    readRecord(entry.detail?.requestBody) ?? readRecord(routeRequest?.bodyCapture);
  if (entry.kind === "routingDecision" && routeRequest) {
    return buildRoutingMetricActions(
      entry,
      routeRequest,
      routeRequestHeaders,
      routeRequestBody,
      localeTag,
      isZh,
      t,
    );
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
