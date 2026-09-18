import { useEffect, useRef, useState } from "react";
import type {
  ApiInvocation,
  ApiInvocationRequestBodyResponse,
  ApiInvocationWorkflowDetailResponse,
  ApiInvocationWorkflowTimelineEntry,
} from "../../lib/api";
import { fetchInvocationRequestBody, fetchInvocationWorkflowDetail } from "../../lib/api";
import { resolveInvocationDisplayStatus } from "../../lib/invocationStatus";
import type {
  AttemptSection,
  GenericSection,
  PayloadFetchState,
} from "./InvocationWorkflowDetailPanel.formatters";
import {
  createIdlePayloadState,
  FALLBACK_CELL,
  formatCurrency,
  formatDurationMs,
  formatOptionalNumber,
  formatOptionalText,
  formatRouteMode,
  resolveStatusMeta,
} from "./InvocationWorkflowDetailPanel.formatters";

export function useInvocationWorkflowDetailLoad(recordId: number) {
  const requestSeqRef = useRef(0);
  const requestBodyFetchSeqRef = useRef(0);
  const [detail, setDetail] = useState<ApiInvocationWorkflowDetailResponse | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    if (!(recordId > 0)) {
      requestSeqRef.current += 1;
      requestBodyFetchSeqRef.current += 1;
      setDetail(null);
      setIsLoading(false);
      setLoadError(null);
      return;
    }

    const requestSeq = requestSeqRef.current + 1;
    requestSeqRef.current = requestSeq;
    setIsLoading(true);
    setLoadError(null);
    void fetchInvocationWorkflowDetail(recordId)
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
        if (requestSeq === requestSeqRef.current) setIsLoading(false);
      });
  }, [recordId]);

  return { detail, isLoading, loadError, requestBodyFetchSeqRef };
}

export function useInvocationWorkflowDetailSelection(
  detail: ApiInvocationWorkflowDetailResponse | null,
  focusedAttemptId: string | null,
  requestBodyFetchSeqRef: { current: number },
  recordId: number,
) {
  const [openBlockId, setOpenBlockId] = useState<string | null>(null);
  const [attemptSection, setAttemptSection] = useState<AttemptSection | null>(null);
  const [genericSection, setGenericSection] = useState<GenericSection | null>(null);
  const [requestBodyState, setRequestBodyState] = useState<
    PayloadFetchState<ApiInvocationRequestBodyResponse>
  >(createIdlePayloadState());

  useEffect(() => {
    requestBodyFetchSeqRef.current += 1;
    setOpenBlockId(null);
    setAttemptSection(null);
    setGenericSection(null);
    setRequestBodyState(createIdlePayloadState());
  }, [requestBodyFetchSeqRef]);

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

  useInvocationWorkflowDetailRequestBody(
    recordId,
    genericSection,
    requestBodyState,
    setRequestBodyState,
    requestBodyFetchSeqRef,
  );

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

  return {
    openBlockId,
    attemptSection,
    genericSection,
    requestBodyState,
    setRequestBodyState,
    toggleAttemptSection,
    toggleGenericSection,
  };
}

export function useInvocationWorkflowDetailRequestBody(
  recordId: number,
  genericSection: GenericSection | null,
  requestBodyState: PayloadFetchState<ApiInvocationRequestBodyResponse>,
  setRequestBodyState: React.Dispatch<
    React.SetStateAction<PayloadFetchState<ApiInvocationRequestBodyResponse>>
  >,
  requestBodyFetchSeqRef: { current: number },
) {
  useEffect(() => {
    if (!(recordId > 0) || genericSection !== "requestBody" || requestBodyState.status !== "idle") {
      return;
    }
    const requestSeq = requestBodyFetchSeqRef.current + 1;
    requestBodyFetchSeqRef.current = requestSeq;
    setRequestBodyState({ status: "loading", data: null, error: null });
    void fetchInvocationRequestBody(recordId)
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
  }, [
    genericSection,
    recordId,
    requestBodyFetchSeqRef,
    requestBodyState.status,
    setRequestBodyState,
  ]);
}

export function buildInvocationWorkflowDetailStatusModel({
  record,
  detail,
  localeTag,
  isZh,
  onOpenUpstreamAccount,
}: {
  record: ApiInvocation;
  detail: ApiInvocationWorkflowDetailResponse;
  localeTag: string;
  isZh: boolean;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
}) {
  const hero = detail.hero;
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
    { label: isZh ? "成本" : "Cost", value: formatCurrency(hero.cost, localeTag) },
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
  const noCandidateAudit = hero.poolRoutingNoCandidateAudit;
  const noCandidateReasonCounts = noCandidateAudit
    ? Object.entries(noCandidateAudit.excludedReasonCounts)
        .filter(([, count]) => Number.isFinite(count) && count > 0)
        .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
    : [];
  return {
    finalStatusMeta,
    summaryRows,
    heroStatusNotes,
    noCandidateAudit,
    noCandidateReasonCounts,
  };
}

export function buildInvocationWorkflowDetailMetricsModel({
  record,
  detail,
  localeTag,
  isZh,
  finalStatusMeta,
}: {
  record: ApiInvocation;
  detail: ApiInvocationWorkflowDetailResponse;
  localeTag: string;
  isZh: boolean;
  finalStatusMeta: ReturnType<typeof resolveStatusMeta>;
}) {
  const hero = detail.hero;
  const timeline = detail.timeline;
  const snapshotMetrics = [
    {
      label: isZh ? "最终结果" : "Final Result",
      value: finalStatusMeta.label,
      variant: finalStatusMeta.variant,
    },
    {
      label: isZh ? "总用时" : "Total Time",
      value: formatDurationMs(hero.totalDurationMs ?? record.tTotalMs, localeTag),
      variant: "primary" as const,
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
    return { key: `${value}-${occurrence}`, value };
  });
  return { snapshotMetrics, modelTrailItems };
}
