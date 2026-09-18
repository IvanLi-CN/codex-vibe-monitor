import type { AttemptDetailSources } from "./InvocationWorkflowDetailPanel.attempt-sources";
import {
  buildStructuredItems,
  FALLBACK_CELL,
  formatByteSize,
  formatOptionalText,
  readNumber,
  readString,
} from "./InvocationWorkflowDetailPanel.formatters";

function buildResponseSummaryItems({
  responseSummary,
  localeTag,
  isZh,
}: Pick<AttemptDetailSources, "responseSummary"> & { localeTag: string; isZh: boolean }) {
  return [
    ...buildStructuredItems(responseSummary, localeTag, isZh, [
      {
        key: "status",
        label: isZh ? "尝试状态" : "Attempt Status",
        monospace: false,
      },
      { key: "phase", label: isZh ? "阶段" : "Phase", monospace: false },
      {
        key: "failureKind",
        label: isZh ? "失败类型" : "Failure Kind",
        monospace: false,
      },
      {
        key: "errorMessage",
        label: isZh ? "错误信息" : "Error Message",
        monospace: false,
      },
      {
        key: "downstreamErrorMessage",
        label: isZh ? "下游错误" : "Downstream Error",
        monospace: false,
      },
      {
        key: "serviceTier",
        label: isZh ? "服务层级" : "Service Tier",
        monospace: false,
      },
      {
        key: "billingServiceTier",
        label: isZh ? "计费层级" : "Billing Tier",
        monospace: false,
      },
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
  ];
}

function buildResponseBodyParsedItems({
  responseBodyParsed,
  localeTag,
  isZh,
}: Pick<AttemptDetailSources, "responseBodyParsed"> & { localeTag: string; isZh: boolean }) {
  return [
    ...buildStructuredItems(responseBodyParsed, localeTag, isZh, [
      { key: "id", label: isZh ? "响应 ID" : "Response ID", monospace: false },
      { key: "object", label: isZh ? "对象类型" : "Object", monospace: false },
      {
        key: "status",
        label: isZh ? "响应状态" : "Body Status",
        monospace: false,
      },
      {
        key: "model",
        label: isZh ? "响应体模型" : "Body Model",
        monospace: false,
      },
      {
        key: "serviceTier",
        label: isZh ? "响应体服务层级" : "Body Tier",
        monospace: false,
      },
      {
        key: "outputItems",
        label: isZh ? "输出项" : "Output Items",
        monospace: false,
      },
      {
        key: "outputTextBlocks",
        label: isZh ? "文本块" : "Output Text Blocks",
        monospace: false,
      },
      {
        key: "toolCalls",
        label: isZh ? "工具调用" : "Tool Calls",
        monospace: false,
      },
      {
        key: "errorCode",
        label: isZh ? "错误码" : "Error Code",
        monospace: false,
      },
      {
        key: "errorMessage",
        label: isZh ? "错误消息" : "Error Message",
        monospace: false,
      },
      {
        key: "usageInputTokens",
        label: isZh ? "输入 Token" : "Input Tokens",
        monospace: false,
      },
      {
        key: "usageOutputTokens",
        label: isZh ? "输出 Token" : "Output Tokens",
        monospace: false,
      },
      {
        key: "usageReasoningTokens",
        label: isZh ? "推理 Token" : "Reasoning Tokens",
        monospace: false,
      },
      {
        key: "usageTotalTokens",
        label: isZh ? "总 Token" : "Total Tokens",
        monospace: false,
      },
    ]),
  ];
}

export function buildResponseParsedItems(
  sources: AttemptDetailSources,
  localeTag: string,
  isZh: boolean,
) {
  return [
    ...buildResponseSummaryItems({ ...sources, localeTag, isZh }),
    ...buildResponseBodyParsedItems({ ...sources, localeTag, isZh }),
  ];
}

export function buildResponseHeaderItems({
  responseHeaderSource,
  localeTag,
  isZh,
  hideNonShortIds,
}: Pick<AttemptDetailSources, "responseHeaderSource"> & {
  localeTag: string;
  isZh: boolean;
  hideNonShortIds: boolean;
}) {
  return buildStructuredItems(responseHeaderSource, localeTag, isZh, [
    { key: "contentEncoding", label: "Content-Encoding", monospace: false },
    {
      key: "contentEncodingChain",
      label: isZh ? "编码链" : "Encoding Chain",
      monospace: false,
    },
    ...(!hideNonShortIds
      ? [
          {
            key: "upstreamRequestId",
            label: "X-Request-ID",
            monospace: false,
          },
        ]
      : []),
    { key: "cvmInvokeId", label: isZh ? "CVM 调用 ID" : "CVM Invoke ID" },
  ]);
}

export function buildResponseDeliveryItems({
  responseDeliverySource,
  localeTag,
  isZh,
}: Pick<AttemptDetailSources, "responseDeliverySource"> & { localeTag: string; isZh: boolean }) {
  return buildStructuredItems(responseDeliverySource, localeTag, isZh, [
    {
      key: "forwardedChunkCount",
      label: isZh ? "转发块数" : "Forwarded Chunks",
      monospace: false,
    },
    {
      key: "forwardedBytes",
      label: isZh ? "转发字节" : "Forwarded Bytes",
      monospace: false,
    },
    {
      key: "usageObserved",
      label: isZh ? "观察到 Usage" : "Usage Observed",
      monospace: false,
    },
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
}

export function buildResponseCaptureSummaryItems({
  responseBodyState,
  responseBodyCaptureSource,
  localeTag,
  isZh,
  responseArchiveAtInvocation,
  responseArchiveAtAttempt,
}: AttemptDetailSources & { localeTag: string; isZh: boolean }) {
  return [
    {
      label: isZh ? "归档" : "Archive",
      value: responseArchiveAtAttempt
        ? isZh
          ? "尝试级"
          : "Attempt"
        : responseArchiveAtInvocation == null || responseArchiveAtInvocation
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
}
