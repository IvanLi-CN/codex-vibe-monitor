import type {
  ApiInvocation,
  ApiInvocationRequestBodyResponse,
  ApiInvocationResponseBodyResponse,
  ApiInvocationWorkflowTimelineEntry,
} from "../../lib/api";
import { AttemptDetailSurface } from "./InvocationWorkflowDetailPanel.attempt-detail-sections";
import {
  buildResponseCaptureSummaryItems,
  buildResponseDeliveryItems,
  buildResponseHeaderItems,
  buildResponseParsedItems,
} from "./InvocationWorkflowDetailPanel.attempt-response-model";
import {
  type Attempt,
  type AttemptDetailSourceArgs,
  type AttemptDetailSources,
  buildAttemptDetailSources,
} from "./InvocationWorkflowDetailPanel.attempt-sources";
import type { AttemptSection, PayloadFetchState } from "./InvocationWorkflowDetailPanel.formatters";
import {
  buildPayloadViewerLabels,
  buildStructuredItems,
  FALLBACK_CELL,
  formatByteSize,
  formatCompactByteSize,
  formatDurationMs,
  formatHttpStatus,
  formatMilliseconds,
  formatOptionalText,
  formatReasoningEffortValue,
  formatRequestCompressionSummary,
  formatResponseMilliseconds,
  formatTimestamp,
  readNumber,
  readString,
} from "./InvocationWorkflowDetailPanel.formatters";

function buildKeyDiagnosticsItems({
  attempt,
  localeTag,
  isZh,
  hideNonShortIds,
}: {
  attempt: Attempt;
  localeTag: string;
  isZh: boolean;
  hideNonShortIds: boolean;
}) {
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
    {
      label: isZh ? "阶段" : "Phase",
      value: formatOptionalText(attempt.phase),
    },
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
      label: "TTFT",
      value: formatDurationMs(attempt.firstTokenMs, localeTag),
    },
    {
      label: isZh ? "流式耗时" : "Stream",
      value: formatResponseMilliseconds(attempt.streamLatencyMs, localeTag),
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

  return keyDiagnosticsItems;
}

function buildTimingItems({
  attempt,
  localeTag,
  record,
  isZh,
}: {
  attempt: Attempt;
  localeTag: string;
  record: ApiInvocation;
  isZh: boolean;
}) {
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
    { label: "TTFT", value: formatDurationMs(attempt.firstTokenMs, localeTag) },
    {
      label: isZh ? "流式" : "Stream",
      value: formatResponseMilliseconds(attempt.streamLatencyMs, localeTag),
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
    {
      label: isZh ? "持久化" : "Persist",
      value: formatMilliseconds(record.tPersistMs, localeTag),
    },
    {
      label: isZh ? "总用时" : "Total",
      value: formatMilliseconds(record.tTotalMs, localeTag),
    },
  ].filter((item) => item.value !== FALLBACK_CELL);

  return timingItems;
}

function buildRequestSummaryItems({
  requestSummary,
  localeTag,
  isZh,
}: Pick<AttemptDetailSources, "requestSummary"> & { localeTag: string; isZh: boolean }) {
  return [
    ...buildStructuredItems(requestSummary, localeTag, isZh, [
      { key: "endpoint", label: isZh ? "端点" : "Endpoint", monospace: false },
      {
        key: "requestModel",
        label: isZh ? "请求模型" : "Request Model",
        monospace: false,
      },
      {
        key: "responseModel",
        label: isZh ? "响应模型" : "Response Model",
        monospace: false,
      },
      {
        key: "requestedServiceTier",
        label: isZh ? "请求服务层级" : "Requested Tier",
        monospace: false,
      },
      {
        key: "reasoningEffort",
        label: isZh ? "推理强度" : "Reasoning Effort",
        monospace: false,
        formatter: formatReasoningEffortValue,
      },
      {
        key: "compactionRequestKind",
        label: isZh ? "请求压缩模式" : "Request Compaction",
        monospace: false,
      },
      {
        key: "imageIntent",
        label: isZh ? "图像工具意图" : "Image Intent",
        monospace: false,
      },
      {
        key: "transport",
        label: isZh ? "传输" : "Transport",
        monospace: false,
      },
    ]),
  ];
}

function buildImageToolItems({
  imageToolRewrite,
  localeTag,
  isZh,
}: Pick<AttemptDetailSources, "imageToolRewrite"> & { localeTag: string; isZh: boolean }) {
  return [
    ...buildStructuredItems(imageToolRewrite, localeTag, isZh, [
      {
        key: "protocol",
        label: isZh ? "图片工具协议" : "Image Tool Protocol",
        monospace: false,
      },
      {
        key: "mode",
        label: isZh ? "图片工具策略" : "Image Tool Policy",
        monospace: false,
      },
      {
        key: "outcome",
        label: isZh ? "图片工具结果" : "Image Tool Outcome",
        monospace: false,
      },
      {
        key: "reason",
        label: isZh ? "图片工具原因" : "Image Tool Reason",
        monospace: false,
      },
    ]),
  ];
}

function buildCodexImagegenItems({
  codexImagegenRewrite,
  localeTag,
  isZh,
}: Pick<AttemptDetailSources, "codexImagegenRewrite"> & { localeTag: string; isZh: boolean }) {
  return [
    ...buildStructuredItems(codexImagegenRewrite, localeTag, isZh, [
      {
        key: "protocol",
        label: isZh ? "Codex 图片协议" : "Codex Image Protocol",
        monospace: false,
      },
      {
        key: "mode",
        label: isZh ? "Codex 图片策略" : "Codex Image Policy",
        monospace: false,
      },
      {
        key: "outcome",
        label: isZh ? "Codex 图片结果" : "Codex Image Outcome",
        monospace: false,
      },
      {
        key: "reason",
        label: isZh ? "Codex 图片原因" : "Codex Image Reason",
        monospace: false,
      },
      {
        key: "hostedRemoved",
        label: isZh ? "移除托管图片工具" : "Hosted Image Removed",
        monospace: false,
      },
      {
        key: "existingSchemaFingerprint",
        label: isZh ? "原 Schema 指纹" : "Existing Schema Fingerprint",
      },
      {
        key: "injectedSchemaFingerprint",
        label: isZh ? "注入 Schema 指纹" : "Injected Schema Fingerprint",
      },
      {
        key: "schemaDiffPaths",
        label: isZh ? "Schema 差异字段" : "Schema Diff Paths",
        monospace: false,
      },
    ]),
  ];
}

function buildRequestBodyParsedItems({
  requestBodyParsed,
  localeTag,
  isZh,
}: Pick<AttemptDetailSources, "requestBodyParsed"> & { localeTag: string; isZh: boolean }) {
  return [
    ...buildStructuredItems(requestBodyParsed, localeTag, isZh, [
      {
        key: "model",
        label: isZh ? "请求体模型" : "Body Model",
        monospace: false,
      },
      {
        key: "stream",
        label: isZh ? "流式返回" : "Streaming",
        monospace: false,
      },
      {
        key: "serviceTier",
        label: isZh ? "请求体服务层级" : "Body Tier",
        monospace: false,
      },
      {
        key: "reasoningEffort",
        label: isZh ? "请求体推理强度" : "Body Reasoning",
        monospace: false,
        formatter: formatReasoningEffortValue,
      },
      {
        key: "maxOutputTokens",
        label: isZh ? "最大输出 Token" : "Max Output Tokens",
        monospace: false,
      },
      {
        key: "temperature",
        label: isZh ? "温度" : "Temperature",
        monospace: false,
      },
      { key: "topP", label: isZh ? "Top P" : "Top P", monospace: false },
      {
        key: "parallelToolCalls",
        label: isZh ? "并行工具调用" : "Parallel Tools",
        monospace: false,
      },
      {
        key: "toolChoice",
        label: isZh ? "工具选择" : "Tool Choice",
        monospace: false,
      },
      { key: "tools", label: isZh ? "工具" : "Tools", monospace: false },
      {
        key: "modalities",
        label: isZh ? "模态" : "Modalities",
        monospace: false,
      },
      {
        key: "inputShape",
        label: isZh ? "输入形态" : "Input Shape",
        monospace: false,
      },
      {
        key: "inputCount",
        label: isZh ? "输入条目" : "Input Count",
        monospace: false,
      },
      {
        key: "textFormat",
        label: isZh ? "返回格式" : "Response Format",
        monospace: false,
      },
    ]),
  ];
}

function buildRequestParsedItems(sources: AttemptDetailSources, localeTag: string, isZh: boolean) {
  return [
    ...buildRequestSummaryItems({ ...sources, localeTag, isZh }),
    ...buildImageToolItems({ ...sources, localeTag, isZh }),
    ...buildCodexImagegenItems({ ...sources, localeTag, isZh }),
    ...buildRequestBodyParsedItems({ ...sources, localeTag, isZh }),
  ];
}

function buildRequestHeaderItems({
  requestHeaderSource,
  localeTag,
  isZh,
}: Pick<AttemptDetailSources, "requestHeaderSource"> & { localeTag: string; isZh: boolean }) {
  const requestHeaderItems = buildStructuredItems(requestHeaderSource, localeTag, isZh, [
    { key: "userAgent", label: "User-Agent", monospace: false },
    { key: "xForwardedFor", label: "X-Forwarded-For", monospace: false },
    { key: "forwarded", label: "Forwarded", monospace: false },
    { key: "xRealIp", label: "X-Real-IP", monospace: false },
  ]);
  return requestHeaderItems;
}

function buildRequestCompressionSummary({
  requestCompression,
  localeTag,
}: Pick<AttemptDetailSources, "requestCompression"> & { localeTag: string }) {
  const requestCompressionSummary = formatRequestCompressionSummary(requestCompression, localeTag);

  return requestCompressionSummary;
}

function buildRequestRoutingItems({
  requestRoutingSource,
  localeTag,
  isZh,
}: Pick<AttemptDetailSources, "requestRoutingSource"> & { localeTag: string; isZh: boolean }) {
  return [
    ...buildStructuredItems(requestRoutingSource, localeTag, isZh, [
      {
        key: "routeMode",
        label: isZh ? "路由模式" : "Route Mode",
        monospace: false,
      },
      {
        key: "upstreamScope",
        label: isZh ? "上游范围" : "Upstream Scope",
        monospace: false,
      },
      { key: "stickyKey", label: "Sticky Key" },
      {
        key: "promptCacheKey",
        label: isZh ? "Prompt Cache Key" : "Prompt Cache Key",
      },
      {
        key: "proxyDisplayName",
        label: isZh ? "代理显示名" : "Proxy Display",
        monospace: false,
      },
      {
        key: "upstreamRouteKey",
        label: isZh ? "上游路由键" : "Upstream Route Key",
      },
      { key: "proxyBindingKey", label: isZh ? "代理绑定" : "Proxy Binding" },
      {
        key: "clientFingerprint",
        label: isZh ? "客户端指纹" : "Client Fingerprint",
      },
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
  ];
}

function buildRequestClientItems({
  requestClientSource,
  localeTag,
  isZh,
}: Pick<AttemptDetailSources, "requestClientSource"> & { localeTag: string; isZh: boolean }) {
  return [
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
}

function buildRequestRoutingSummaryItems(
  sources: AttemptDetailSources,
  localeTag: string,
  isZh: boolean,
) {
  return [
    ...buildRequestRoutingItems({ ...sources, localeTag, isZh }),
    ...buildRequestClientItems({ ...sources, localeTag, isZh }),
  ];
}

function buildRequestCaptureSummaryItems({
  requestBodyState,
  localeTag,
  isZh,
  requestArchiveAtInvocation,
}: AttemptDetailSources & { localeTag: string; isZh: boolean }) {
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
  return requestCaptureSummaryItems;
}

function buildRequestCompressionItems({
  requestCompression,
  requestCompressionSummary,
  localeTag,
  isZh,
}: Pick<AttemptDetailSources, "requestCompression"> & {
  requestCompressionSummary: string;
  localeTag: string;
  isZh: boolean;
}) {
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

  return requestCompressionItems;
}

function buildAttemptDetailModel({
  record,
  localeTag,
  isZh,
  hideNonShortIds,
  ...sourceArgs
}: AttemptDetailSourceArgs & {
  record: ApiInvocation;
  localeTag: string;
  isZh: boolean;
  hideNonShortIds: boolean;
}) {
  const sources = buildAttemptDetailSources(sourceArgs);
  const requestCompressionSummary = buildRequestCompressionSummary({ ...sources, localeTag });
  return {
    ...sources,
    labels: buildPayloadViewerLabels(isZh),
    isZh,
    localeTag,
    keyDiagnosticsItems: buildKeyDiagnosticsItems({ ...sources, localeTag, isZh, hideNonShortIds }),
    timingItems: buildTimingItems({ ...sources, localeTag, record, isZh }),
    requestParsedItems: buildRequestParsedItems(sources, localeTag, isZh),
    requestHeaderItems: buildRequestHeaderItems({ ...sources, localeTag, isZh }),
    requestCompressionSummary,
    requestRoutingItems: buildRequestRoutingSummaryItems(sources, localeTag, isZh),
    requestCaptureSummaryItems: buildRequestCaptureSummaryItems({ ...sources, localeTag, isZh }),
    requestCompressionItems: buildRequestCompressionItems({
      ...sources,
      requestCompressionSummary,
      localeTag,
      isZh,
    }),
    responseParsedItems: buildResponseParsedItems(sources, localeTag, isZh),
    responseHeaderItems: buildResponseHeaderItems({ ...sources, localeTag, isZh, hideNonShortIds }),
    responseDeliveryItems: buildResponseDeliveryItems({ ...sources, localeTag, isZh }),
    responseCaptureSummaryItems: buildResponseCaptureSummaryItems({ ...sources, localeTag, isZh }),
    requestBodyContent: sourceArgs.requestBodyState.data?.bodyText?.trim() ?? "",
    responseBodyContent: sourceArgs.responseBodyState.data?.bodyText?.trim() ?? "",
  };
}

export type AttemptDetailModel = ReturnType<typeof buildAttemptDetailModel>;

export function AttemptDetail({
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
  const model = buildAttemptDetailModel({
    record,
    attempt,
    localeTag,
    isZh,
    hideNonShortIds,
    requestBodyState,
    responseBodyState,
  });
  return <AttemptDetailSurface model={model} activeSection={activeSection} />;
}
