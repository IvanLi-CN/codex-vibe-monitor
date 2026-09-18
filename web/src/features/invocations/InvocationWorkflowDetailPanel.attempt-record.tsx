import {
  type Dispatch,
  type MutableRefObject,
  type SetStateAction,
  useEffect,
  useRef,
  useState,
} from "react";
import type {
  ApiInvocation,
  ApiInvocationRequestBodyResponse,
  ApiInvocationResponseBodyResponse,
  ApiInvocationWorkflowTimelineEntry,
} from "../../lib/api";
import {
  fetchInvocationAttemptResponseBody,
  fetchInvocationRequestBody,
  fetchInvocationResponseBody,
} from "../../lib/api";
import { cn } from "../../lib/utils";
import { AttemptDetail } from "./InvocationWorkflowDetailPanel.attempt-detail";
import type { AttemptSection, PayloadFetchState } from "./InvocationWorkflowDetailPanel.formatters";
import {
  createIdlePayloadState,
  isRequestSection,
  isResponseSection,
  readBoolean,
  readRecord,
  readString,
} from "./InvocationWorkflowDetailPanel.formatters";
import { TimelineSummary } from "./InvocationWorkflowDetailPanel.timeline-summary";

export interface InvocationWorkflowAttemptRecordProps {
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

export function useInvocationRequestBody(
  recordId: number,
  currentSection: AttemptSection | null,
  requestBodyState: PayloadFetchState<ApiInvocationRequestBodyResponse>,
  setRequestBodyState: Dispatch<
    SetStateAction<PayloadFetchState<ApiInvocationRequestBodyResponse>>
  >,
  requestBodyFetchSeqRef: MutableRefObject<number>,
) {
  useEffect(() => {
    if (!(recordId > 0) || !currentSection || !isRequestSection(currentSection)) return;
    if (requestBodyState.status !== "idle") return;
    const requestSeq = requestBodyFetchSeqRef.current + 1;
    requestBodyFetchSeqRef.current = requestSeq;
    setRequestBodyState({ status: "loading", data: null, error: null });
    void fetchInvocationRequestBody(recordId)
      .then((data) => {
        if (requestSeq === requestBodyFetchSeqRef.current) {
          setRequestBodyState({ status: "loaded", data, error: null });
        }
      })
      .catch((error) => {
        if (requestSeq === requestBodyFetchSeqRef.current) {
          setRequestBodyState({
            status: "error",
            data: null,
            error: error instanceof Error ? error.message : String(error),
          });
        }
      });
  }, [
    currentSection,
    recordId,
    requestBodyFetchSeqRef,
    requestBodyState.status,
    setRequestBodyState,
  ]);
}

export function useInvocationResponseBody({
  recordId,
  currentSection,
  attemptPublicId,
  invocationResponseFallbackAllowed,
  responseBodyUnavailableReason,
  responseBodyState,
  setResponseBodyState,
  responseBodyFetchSeqRef,
}: {
  recordId: number;
  currentSection: AttemptSection | null;
  attemptPublicId: string | null;
  invocationResponseFallbackAllowed: boolean | null;
  responseBodyUnavailableReason: string;
  responseBodyState: PayloadFetchState<ApiInvocationResponseBodyResponse>;
  setResponseBodyState: Dispatch<
    SetStateAction<PayloadFetchState<ApiInvocationResponseBodyResponse>>
  >;
  responseBodyFetchSeqRef: MutableRefObject<number>;
}) {
  useEffect(() => {
    if (!(recordId > 0) || !currentSection || !isResponseSection(currentSection)) return;
    if (responseBodyState.status !== "idle") return;
    const requestSeq = responseBodyFetchSeqRef.current + 1;
    responseBodyFetchSeqRef.current = requestSeq;
    if (!attemptPublicId && !invocationResponseFallbackAllowed) {
      setResponseBodyState({
        status: "loaded",
        data: { available: false, unavailableReason: responseBodyUnavailableReason },
        error: null,
      });
      return;
    }
    setResponseBodyState({ status: "loading", data: null, error: null });
    const fetchResponseBody = attemptPublicId
      ? fetchInvocationAttemptResponseBody(recordId, attemptPublicId)
      : fetchInvocationResponseBody(recordId);
    void fetchResponseBody
      .then((data) => {
        if (requestSeq === responseBodyFetchSeqRef.current) {
          setResponseBodyState({ status: "loaded", data, error: null });
        }
      })
      .catch((error) => {
        if (requestSeq === responseBodyFetchSeqRef.current) {
          setResponseBodyState({
            status: "error",
            data: null,
            error: error instanceof Error ? error.message : String(error),
          });
        }
      });
  }, [
    attemptPublicId,
    currentSection,
    invocationResponseFallbackAllowed,
    recordId,
    responseBodyFetchSeqRef,
    responseBodyState.status,
    responseBodyUnavailableReason,
    setResponseBodyState,
  ]);
}

export function InvocationWorkflowAttemptRecordView({
  record,
  entry,
  localeTag,
  isZh,
  summaryIdentity,
  focused,
  className,
  containerRef,
  testId,
  currentOpen,
  currentSection,
  handleSelectSection,
  requestBodyState,
  responseBodyState,
  hideNonShortIds,
}: InvocationWorkflowAttemptRecordProps & {
  currentOpen: boolean;
  currentSection: AttemptSection | null;
  handleSelectSection: (section: AttemptSection) => void;
  requestBodyState: PayloadFetchState<ApiInvocationRequestBodyResponse>;
  responseBodyState: PayloadFetchState<ApiInvocationResponseBodyResponse>;
}) {
  if (!entry.attempt) return null;
  return (
    <div
      ref={containerRef}
      className={cn(
        "invocation-workflow-attempt min-w-0 max-w-full scroll-mt-4 rounded-[1.125rem] border border-transparent transition-[background-color,border-color,box-shadow] duration-200",
        focused && "invocation-workflow-attempt--focused bg-primary/8",
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

export function InvocationWorkflowAttemptRecord({
  record,
  entry,
  localeTag,
  isZh,
  summaryIdentity,
  focused = false,
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
  const attemptPublicId = entry.attempt?.attemptId?.trim() || null;
  const responseBodyCaptureSummary = readRecord(entry.attempt?.responseSummary);
  const responseBodyCapture = readRecord(responseBodyCaptureSummary?.responseBodyCapture);
  const invocationResponseFallbackAllowed = readBoolean(
    responseBodyCapture?.availableAtInvocationLevel,
  );
  const responseBodyUnavailableReason =
    readString(responseBodyCapture?.unavailableReason) ?? "attempt_response_body_not_captured";

  useEffect(() => {
    requestBodyFetchSeqRef.current += 1;
    responseBodyFetchSeqRef.current += 1;
    setRequestBodyState(createIdlePayloadState());
    setResponseBodyState(createIdlePayloadState());
  }, []);

  useEffect(() => {
    if (isControlled || !focused || !defaultSection) return;
    setInternalSection(defaultSection);
  }, [defaultSection, focused, isControlled]);

  useInvocationRequestBody(
    record.id,
    currentSection,
    requestBodyState,
    setRequestBodyState,
    requestBodyFetchSeqRef,
  );
  useInvocationResponseBody({
    recordId: record.id,
    currentSection,
    attemptPublicId,
    invocationResponseFallbackAllowed,
    responseBodyUnavailableReason,
    responseBodyState,
    setResponseBodyState,
    responseBodyFetchSeqRef,
  });

  const handleSelectSection = (section: AttemptSection) => {
    if (isControlled) {
      onSelectSection?.(section);
      return;
    }
    setInternalSection((current) => (current === section ? null : section));
  };

  return (
    <InvocationWorkflowAttemptRecordView
      record={record}
      entry={entry}
      localeTag={localeTag}
      isZh={isZh}
      summaryIdentity={summaryIdentity}
      focused={focused}
      className={className}
      containerRef={containerRef}
      testId={testId}
      currentOpen={currentOpen === true}
      currentSection={currentSection}
      handleSelectSection={(section) => handleSelectSection(section as AttemptSection)}
      requestBodyState={requestBodyState}
      responseBodyState={responseBodyState}
      hideNonShortIds={hideNonShortIds}
    />
  );
}
