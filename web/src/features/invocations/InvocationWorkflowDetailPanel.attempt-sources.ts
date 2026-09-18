import type {
  ApiInvocationRequestBodyResponse,
  ApiInvocationResponseBodyResponse,
  ApiInvocationWorkflowTimelineEntry,
} from "../../lib/api";
import type { PayloadFetchState } from "./InvocationWorkflowDetailPanel.formatters";
import {
  extractRequestBusinessSnapshot,
  extractResponseBusinessSnapshot,
  readAttemptUsageAudit,
  readBoolean,
  readRecord,
  readString,
} from "./InvocationWorkflowDetailPanel.formatters";

export type Attempt = NonNullable<ApiInvocationWorkflowTimelineEntry["attempt"]>;

export type AttemptDetailSourceArgs = {
  attempt: Attempt;
  requestBodyState: PayloadFetchState<ApiInvocationRequestBodyResponse>;
  responseBodyState: PayloadFetchState<ApiInvocationResponseBodyResponse>;
};

export function buildAttemptDetailSources({
  attempt,
  requestBodyState,
  responseBodyState,
}: AttemptDetailSourceArgs) {
  const requestSummary = readRecord(attempt.requestSummary);
  const imageToolRewrite = readRecord(requestSummary?.imageToolRewrite);
  const codexImagegenRewrite = readRecord(requestSummary?.codexImagegenRewrite);
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
  const responseArchiveAtAttempt = readBoolean(responseBodyCaptureSource?.availableAtAttemptLevel);
  const responseBodyUnavailableReason =
    responseBodyState.data?.unavailableReason ??
    readString(responseBodyCaptureSource?.unavailableReason);

  return {
    attempt,
    requestBodyState,
    responseBodyState,
    requestSummary,
    imageToolRewrite,
    codexImagegenRewrite,
    responseSummary,
    usageAudit,
    requestBodyParsed,
    responseBodyParsed,
    requestHeaderSource,
    requestCompression,
    requestRoutingSource,
    requestClientSource,
    responseHeaderSource,
    responseDeliverySource,
    responseBodyCaptureSource,
    requestArchiveAtInvocation,
    responseArchiveAtInvocation,
    responseArchiveAtAttempt,
    responseBodyUnavailableReason,
  };
}

export type AttemptDetailSources = ReturnType<typeof buildAttemptDetailSources>;
