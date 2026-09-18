import type { AttemptDetailModel } from "./InvocationWorkflowDetailPanel.attempt-detail";
import type { AttemptSection } from "./InvocationWorkflowDetailPanel.formatters";
import { formatPayloadUnavailableReason } from "./InvocationWorkflowDetailPanel.formatters";
import {
  AttemptUsageAuditPanel,
  DetailFrame,
  DetailInfoPanel,
  DetailMetaStrip,
  PayloadNotice,
} from "./InvocationWorkflowDetailPanel.primitives";
import { StructuredPayloadViewer } from "./StructuredPayloadViewer";

function AttemptTimingSection({
  model,
  activeSection,
}: {
  model: AttemptDetailModel;
  activeSection: AttemptSection;
}) {
  const { keyDiagnosticsItems, timingItems, isZh } = model;
  return (
    <>
      {activeSection === "timing" ? (
        <>
          <DetailInfoPanel
            title={isZh ? "关键诊断" : "Key diagnostics"}
            items={keyDiagnosticsItems}
          />
          <DetailInfoPanel title={isZh ? "时间细分" : "Timing breakdown"} items={timingItems} />
        </>
      ) : null}
    </>
  );
}

function AttemptRequestParsedSection({
  model,
  activeSection,
}: {
  model: AttemptDetailModel;
  activeSection: AttemptSection;
}) {
  const { requestParsedItems, requestRoutingItems, requestBodyState, isZh } = model;
  return (
    <>
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
    </>
  );
}

function AttemptRequestHeadersSection({
  model,
  activeSection,
}: {
  model: AttemptDetailModel;
  activeSection: AttemptSection;
}) {
  const { requestHeaderItems, requestRoutingItems, requestCaptureSummaryItems, isZh } = model;
  return (
    <>
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
    </>
  );
}

function AttemptRequestBodySection({
  model,
  activeSection,
}: {
  model: AttemptDetailModel;
  activeSection: AttemptSection;
}) {
  const {
    requestCompressionItems,
    requestCaptureSummaryItems,
    requestBodyState,
    requestBodyContent,
    labels,
    isZh,
  } = model;
  return (
    <>
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
    </>
  );
}

function AttemptResponseParsedSection({
  model,
  activeSection,
}: {
  model: AttemptDetailModel;
  activeSection: AttemptSection;
}) {
  const {
    responseParsedItems,
    responseDeliveryItems,
    usageAudit,
    responseBodyState,
    isZh,
    localeTag,
  } = model;
  return (
    <>
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
    </>
  );
}

function AttemptResponseHeadersSection({
  model,
  activeSection,
}: {
  model: AttemptDetailModel;
  activeSection: AttemptSection;
}) {
  const { responseHeaderItems, responseDeliveryItems, responseCaptureSummaryItems, isZh } = model;
  return (
    <>
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
    </>
  );
}

function AttemptResponseBodySection({
  model,
  activeSection,
}: {
  model: AttemptDetailModel;
  activeSection: AttemptSection;
}) {
  const {
    responseCaptureSummaryItems,
    responseBodyState,
    responseBodyContent,
    labels,
    responseBodyUnavailableReason,
    isZh,
  } = model;
  return (
    <>
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
    </>
  );
}

export function AttemptDetailSurface({
  model,
  activeSection,
}: {
  model: AttemptDetailModel;
  activeSection: AttemptSection;
}) {
  return (
    <DetailFrame>
      <AttemptTimingSection model={model} activeSection={activeSection} />
      <AttemptRequestParsedSection model={model} activeSection={activeSection} />
      <AttemptRequestHeadersSection model={model} activeSection={activeSection} />
      <AttemptRequestBodySection model={model} activeSection={activeSection} />
      <AttemptResponseParsedSection model={model} activeSection={activeSection} />
      <AttemptResponseHeadersSection model={model} activeSection={activeSection} />
      <AttemptResponseBodySection model={model} activeSection={activeSection} />
    </DetailFrame>
  );
}
