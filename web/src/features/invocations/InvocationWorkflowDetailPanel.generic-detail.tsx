import type {
  ApiInvocationRequestBodyResponse,
  ApiInvocationWorkflowTimelineEntry,
} from "../../lib/api";
import type { GenericSection, PayloadFetchState } from "./InvocationWorkflowDetailPanel.formatters";
import {
  buildPayloadViewerLabels,
  buildStructuredItems,
  FALLBACK_CELL,
  formatByteSize,
  formatOptionalText,
  formatPayloadUnavailableReason,
  formatReasoningEffortValue,
  readBoolean,
  readNumber,
  readRecord,
  readString,
  stringifyStructuredValue,
} from "./InvocationWorkflowDetailPanel.formatters";
import {
  DetailFrame,
  DetailInfoPanel,
  DetailMetaStrip,
  PayloadNotice,
} from "./InvocationWorkflowDetailPanel.primitives";
import { StructuredPayloadViewer } from "./StructuredPayloadViewer";

type GenericDetailSources = {
  routeRequest: Record<string, unknown> | null;
  routeRequestHeaders: Record<string, unknown> | null;
  routeRequestRouting: Record<string, unknown> | null;
  routeRequestClient: Record<string, unknown> | null;
  routeRequestAccount: Record<string, unknown> | null;
  routeRequestCompression: Record<string, unknown> | null;
  routeRequestBody: Record<string, unknown> | null;
};

function readGenericDetailSources(entry: ApiInvocationWorkflowTimelineEntry): GenericDetailSources {
  const routeRequest = readRecord(entry.detail?.request);
  return {
    routeRequest,
    routeRequestHeaders:
      readRecord(entry.detail?.requestHeaders) ?? readRecord(routeRequest?.headers),
    routeRequestRouting: readRecord(routeRequest?.routing),
    routeRequestClient: readRecord(routeRequest?.client),
    routeRequestAccount: readRecord(routeRequest?.account),
    routeRequestCompression: readRecord(routeRequest?.compression),
    routeRequestBody:
      readRecord(entry.detail?.requestBody) ?? readRecord(routeRequest?.bodyCapture),
  };
}

function buildRouteRequestParsedItems(
  sources: GenericDetailSources,
  localeTag: string,
  isZh: boolean,
) {
  return [
    ...buildStructuredItems(sources.routeRequest, localeTag, isZh, [
      { key: "endpoint", label: isZh ? "端点" : "Endpoint", monospace: false },
      { key: "requestModel", label: isZh ? "请求模型" : "Request Model", monospace: false },
      { key: "responseModel", label: isZh ? "响应模型" : "Response Model", monospace: false },
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
      { key: "imageIntent", label: isZh ? "图像工具意图" : "Image Intent", monospace: false },
      { key: "transport", label: isZh ? "传输" : "Transport", monospace: false },
      { key: "promptCacheKey", label: "Prompt Cache Key" },
      { key: "stickyKey", label: "Sticky Key" },
      { key: "requesterIp", label: isZh ? "请求 IP" : "Requester IP", monospace: false },
    ]),
    ...buildStructuredItems(sources.routeRequestAccount, localeTag, isZh, [
      { key: "id", label: isZh ? "账号 ID" : "Account ID", monospace: false },
      { key: "name", label: isZh ? "账号" : "Account", monospace: false },
    ]),
    ...buildStructuredItems(sources.routeRequestCompression, localeTag, isZh, [
      { key: "algorithm", label: isZh ? "压缩算法" : "Compression", monospace: false },
      { key: "mode", label: isZh ? "压缩模式" : "Compression Mode", monospace: false },
    ]),
  ];
}

function buildRouteRequestRoutingItems(
  sources: GenericDetailSources,
  localeTag: string,
  isZh: boolean,
) {
  return [
    ...buildStructuredItems(sources.routeRequestRouting, localeTag, isZh, [
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
    ...buildStructuredItems(sources.routeRequestClient, localeTag, isZh, [
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

function buildRouteBodySummaryItems(
  sources: GenericDetailSources,
  requestBodyState: PayloadFetchState<ApiInvocationRequestBodyResponse>,
  localeTag: string,
  isZh: boolean,
) {
  const archivedAtInvocation = readBoolean(sources.routeRequestBody?.availableAtInvocationLevel);
  const bodyTruncated =
    requestBodyState.data?.bodyTruncated ?? readBoolean(sources.routeRequestBody?.truncated);
  return [
    {
      label: isZh ? "归档" : "Archive",
      value:
        archivedAtInvocation == null
          ? FALLBACK_CELL
          : archivedAtInvocation
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
        requestBodyState.data?.bodySize ?? readNumber(sources.routeRequestBody?.size),
        localeTag,
      ),
      monospace: false,
    },
    {
      label: isZh ? "详情" : "Detail",
      value: formatOptionalText(
        requestBodyState.data?.detailLevel ?? readString(sources.routeRequestBody?.detailLevel),
      ),
      monospace: false,
    },
    {
      label: isZh ? "截断" : "Truncated",
      value:
        bodyTruncated == null
          ? FALLBACK_CELL
          : bodyTruncated
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
        requestBodyState.data?.bodyTruncatedReason ??
          readString(sources.routeRequestBody?.truncatedReason),
      ),
      monospace: false,
      fullWidth: true,
    },
    {
      label: isZh ? "裁剪原因" : "Prune Reason",
      value: formatOptionalText(
        requestBodyState.data?.detailPruneReason ??
          readString(sources.routeRequestBody?.detailPruneReason),
      ),
      monospace: false,
      fullWidth: true,
    },
  ].filter((item) => item.value !== FALLBACK_CELL);
}

function buildGenericDetailModel(
  entry: ApiInvocationWorkflowTimelineEntry,
  requestBodyState: PayloadFetchState<ApiInvocationRequestBodyResponse>,
  localeTag: string,
  isZh: boolean,
) {
  const sources = readGenericDetailSources(entry);
  return {
    labels: buildPayloadViewerLabels(isZh),
    detailContent: stringifyStructuredValue(entry.detail ?? undefined),
    bodyText: entry.responseBody?.bodyText?.trim() ?? "",
    routeRequestParsedItems: buildRouteRequestParsedItems(sources, localeTag, isZh),
    routeRequestRoutingItems: buildRouteRequestRoutingItems(sources, localeTag, isZh),
    routeHeaderItems: buildStructuredItems(sources.routeRequestHeaders, localeTag, isZh, [
      { key: "userAgent", label: "User-Agent", monospace: false },
      { key: "xForwardedFor", label: "X-Forwarded-For", monospace: false },
      { key: "forwarded", label: "Forwarded", monospace: false },
      { key: "xRealIp", label: "X-Real-IP", monospace: false },
    ]),
    routeBodySummaryItems: buildRouteBodySummaryItems(sources, requestBodyState, localeTag, isZh),
    routeRequestBodyContent: requestBodyState.data?.bodyText?.trim() ?? "",
  };
}

type GenericDetailModel = ReturnType<typeof buildGenericDetailModel>;

function GenericRoutingRequestSection({
  model,
  entry,
  activeSection,
  isZh,
}: {
  model: GenericDetailModel;
  entry: ApiInvocationWorkflowTimelineEntry;
  activeSection: GenericSection;
  isZh: boolean;
}) {
  if (entry.kind !== "routingDecision" || activeSection !== "request") return null;
  return (
    <>
      <DetailInfoPanel
        title={isZh ? "解析后的请求" : "Parsed request"}
        items={model.routeRequestParsedItems}
      />
      <DetailInfoPanel
        title={isZh ? "路由与会话信号" : "Routing and session"}
        items={model.routeRequestRoutingItems}
      />
      <DetailMetaStrip items={model.routeBodySummaryItems} />
    </>
  );
}

function GenericRoutingHeadersSection({
  model,
  entry,
  activeSection,
  isZh,
}: {
  model: GenericDetailModel;
  entry: ApiInvocationWorkflowTimelineEntry;
  activeSection: GenericSection;
  isZh: boolean;
}) {
  if (entry.kind !== "routingDecision" || activeSection !== "requestHeaders") return null;
  return (
    <>
      <DetailInfoPanel title={isZh ? "请求头" : "Request headers"} items={model.routeHeaderItems} />
      <DetailInfoPanel
        title={isZh ? "路由与会话信号" : "Routing and session"}
        items={model.routeRequestRoutingItems}
      />
    </>
  );
}

function GenericRoutingBodySection({
  model,
  entry,
  activeSection,
  requestBodyState,
  isZh,
}: {
  model: GenericDetailModel;
  entry: ApiInvocationWorkflowTimelineEntry;
  activeSection: GenericSection;
  requestBodyState: PayloadFetchState<ApiInvocationRequestBodyResponse>;
  isZh: boolean;
}) {
  if (entry.kind !== "routingDecision" || activeSection !== "requestBody") return null;
  return (
    <>
      <DetailMetaStrip items={model.routeBodySummaryItems} />
      {requestBodyState.status === "loading" ? (
        <PayloadNotice>{isZh ? "加载请求体…" : "Loading request body…"}</PayloadNotice>
      ) : requestBodyState.status === "error" ? (
        <PayloadNotice tone="error">
          {isZh ? "请求体加载失败：" : "Failed to load request body: "}
          {requestBodyState.error}
        </PayloadNotice>
      ) : requestBodyState.data?.available && model.routeRequestBodyContent ? (
        <StructuredPayloadViewer value={model.routeRequestBodyContent} labels={model.labels} />
      ) : (
        <PayloadNotice tone="warning">
          {isZh ? "请求体不可用：" : "Request body unavailable: "}
          {formatPayloadUnavailableReason(requestBodyState.data?.unavailableReason, isZh)}
        </PayloadNotice>
      )}
    </>
  );
}

function GenericJsonSection({
  model,
  activeSection,
  isZh,
}: {
  model: GenericDetailModel;
  activeSection: GenericSection;
  isZh: boolean;
}) {
  if (activeSection !== "json") return null;
  return model.detailContent ? (
    <StructuredPayloadViewer value={model.detailContent} labels={model.labels} />
  ) : (
    <div className="rounded-xl border border-base-300/70 bg-base-100/80 px-3 py-3 text-sm text-base-content/62">
      {isZh ? "无 JSON" : "No JSON"}
    </div>
  );
}

function GenericResponseBodySection({
  model,
  entry,
  activeSection,
  isZh,
}: {
  model: GenericDetailModel;
  entry: ApiInvocationWorkflowTimelineEntry;
  activeSection: GenericSection;
  isZh: boolean;
}) {
  if (activeSection !== "body" || !entry.responseBody) return null;
  return entry.responseBody.available && model.bodyText ? (
    <StructuredPayloadViewer value={model.bodyText} labels={model.labels} />
  ) : (
    <div className="rounded-xl border border-warning/25 bg-warning/8 px-3 py-3 text-sm text-base-content/72">
      {isZh ? "响应体不可用：" : "Response body unavailable: "}
      {formatPayloadUnavailableReason(entry.responseBody.unavailableReason, isZh)}
    </div>
  );
}

export function GenericDetail({
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
  const model = buildGenericDetailModel(entry, requestBodyState, localeTag, isZh);
  return (
    <DetailFrame>
      <GenericRoutingRequestSection
        model={model}
        entry={entry}
        activeSection={activeSection}
        isZh={isZh}
      />
      <GenericRoutingHeadersSection
        model={model}
        entry={entry}
        activeSection={activeSection}
        isZh={isZh}
      />
      <GenericRoutingBodySection
        model={model}
        entry={entry}
        activeSection={activeSection}
        requestBodyState={requestBodyState}
        isZh={isZh}
      />
      <GenericJsonSection model={model} activeSection={activeSection} isZh={isZh} />
      <GenericResponseBodySection
        model={model}
        entry={entry}
        activeSection={activeSection}
        isZh={isZh}
      />
    </DetailFrame>
  );
}
