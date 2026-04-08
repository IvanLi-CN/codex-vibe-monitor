import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import type {
  ApiInvocation,
  ApiInvocationRecordDetailResponse,
  ApiInvocationResponseBodyResponse,
} from "../lib/api";
import {
  fetchInvocationRecordDetail,
  fetchInvocationRecords,
  fetchInvocationResponseBody,
} from "../lib/api";
import {
  formatDashboardWorkingConversationSequenceId,
  type DashboardWorkingConversationInvocationSelection,
} from "../lib/dashboardWorkingConversations";
import { resolveInvocationDisplayStatus } from "../lib/invocationStatus";
import { useTranslation } from "../i18n";
import { AccountDetailDrawerShell } from "./AccountDetailDrawerShell";
import { AppIcon } from "./AppIcon";
import {
  FALLBACK_CELL,
  InvocationExpandedDetails,
  buildInvocationDetailViewModel,
  renderEndpointSummary,
  useInvocationPoolAttempts,
} from "./invocation-details-shared";
import { Alert } from "./ui/alert";
import { Badge } from "./ui/badge";
import { Spinner } from "./ui/spinner";

interface DashboardInvocationDetailDrawerProps {
  open: boolean;
  selection: DashboardWorkingConversationInvocationSelection | null;
  onClose: () => void;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
}

type StatusMeta = {
  variant: "default" | "secondary" | "success" | "warning" | "error";
  labelKey?: string;
  label?: string;
};

function formatStatusLabel(status: string) {
  const normalized = status.trim();
  if (!normalized) return null;
  const lower = normalized.toLowerCase();
  if (lower.startsWith("http_")) {
    const code = lower.slice("http_".length);
    if (/^\d{3}$/.test(code)) return `HTTP ${code}`;
    return normalized.toUpperCase().replace("_", " ");
  }
  return normalized;
}

function resolveStatusMeta(status?: string | null): StatusMeta {
  const raw = (status ?? "").trim();
  const lower = raw.toLowerCase();
  if (lower === "success" || lower === "completed") {
    return { variant: "success", labelKey: "table.status.success" };
  }
  if (lower === "failed") {
    return { variant: "error", labelKey: "table.status.failed" };
  }
  if (lower === "interrupted") {
    return { variant: "error", labelKey: "table.status.interrupted" };
  }
  if (lower === "running") {
    return { variant: "default", labelKey: "table.status.running" };
  }
  if (lower === "pending") {
    return { variant: "warning", labelKey: "table.status.pending" };
  }
  if (!raw) {
    return { variant: "secondary", labelKey: "table.status.unknown" };
  }
  if (lower.startsWith("http_4")) {
    return { variant: "warning", label: formatStatusLabel(raw) ?? raw };
  }
  if (lower.startsWith("http_5")) {
    return { variant: "error", label: formatStatusLabel(raw) ?? raw };
  }
  if (lower.startsWith("http_")) {
    return { variant: "secondary", label: formatStatusLabel(raw) ?? raw };
  }
  return { variant: "secondary", label: raw };
}

function isAbnormalRecord(record: ApiInvocation) {
  const failureClass = record.failureClass?.trim().toLowerCase();
  if (
    failureClass === "service_failure" ||
    failureClass === "client_failure" ||
    failureClass === "client_abort"
  ) {
    return true;
  }
  return (
    (resolveInvocationDisplayStatus(record) ?? "").trim().toLowerCase() ===
    "failed"
  );
}

function formatOccurredAtLabel(value: string, formatter: Intl.DateTimeFormat) {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value || FALLBACK_CELL;
  return formatter.format(parsed);
}

function SummaryCard({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="rounded-xl border border-base-300/70 bg-base-100/65 p-3">
      <div className="mb-2 text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/60">
        {label}
      </div>
      {children}
    </div>
  );
}

export function DashboardInvocationDetailDrawer({
  open,
  selection,
  onClose,
  onOpenUpstreamAccount,
}: DashboardInvocationDetailDrawerProps) {
  const { t, locale } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const titleId = useId();
  const requestSeqRef = useRef(0);
  const abnormalSeqRef = useRef(0);
  const [fullRecord, setFullRecord] = useState<ApiInvocation | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [recordDetail, setRecordDetail] =
    useState<ApiInvocationRecordDetailResponse | null>(null);
  const [recordDetailLoading, setRecordDetailLoading] = useState(false);
  const [recordDetailError, setRecordDetailError] = useState<string | null>(
    null,
  );
  const [responseBody, setResponseBody] =
    useState<ApiInvocationResponseBodyResponse | null>(null);
  const [responseBodyLoading, setResponseBodyLoading] = useState(false);
  const [responseBodyError, setResponseBodyError] = useState<string | null>(
    null,
  );

  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag),
    [localeTag],
  );
  const currencyFormatter = useMemo(
    () =>
      new Intl.NumberFormat(localeTag, {
        style: "currency",
        currency: "USD",
        minimumFractionDigits: 4,
        maximumFractionDigits: 4,
      }),
    [localeTag],
  );
  const dateTimeFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(localeTag, {
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
        hour12: false,
      }),
    [localeTag],
  );

  useEffect(() => {
    if (!open || !selection) {
      requestSeqRef.current += 1;
      abnormalSeqRef.current += 1;
      setFullRecord(null);
      setIsLoading(false);
      setLoadError(null);
      setRecordDetail(null);
      setRecordDetailLoading(false);
      setRecordDetailError(null);
      setResponseBody(null);
      setResponseBodyLoading(false);
      setResponseBodyError(null);
      return;
    }

    const requestSeq = requestSeqRef.current + 1;
    requestSeqRef.current = requestSeq;
    setFullRecord(null);
    setIsLoading(true);
    setLoadError(null);
    setRecordDetail(null);
    setRecordDetailLoading(false);
    setRecordDetailError(null);
    setResponseBody(null);
    setResponseBodyLoading(false);
    setResponseBodyError(null);

    void fetchInvocationRecords({
      requestId: selection.invocation.record.invokeId,
      pageSize: 1,
      sortBy: "occurredAt",
      sortOrder: "desc",
    })
      .then((response) => {
        if (requestSeq !== requestSeqRef.current) return;
        const exactRecord =
          response.records.find(
            (record) =>
              record.invokeId === selection.invocation.record.invokeId,
          ) ?? null;
        setFullRecord(exactRecord);
      })
      .catch((error) => {
        if (requestSeq !== requestSeqRef.current) return;
        setLoadError(error instanceof Error ? error.message : String(error));
      })
      .finally(() => {
        if (requestSeq === requestSeqRef.current) {
          setIsLoading(false);
        }
      });
  }, [open, selection]);

  useEffect(() => {
    if (!open || !fullRecord || !isAbnormalRecord(fullRecord)) {
      abnormalSeqRef.current += 1;
      setRecordDetail(null);
      setRecordDetailLoading(false);
      setRecordDetailError(null);
      setResponseBody(null);
      setResponseBodyLoading(false);
      setResponseBodyError(null);
      return;
    }

    const abnormalSeq = abnormalSeqRef.current + 1;
    abnormalSeqRef.current = abnormalSeq;
    setRecordDetail(null);
    setRecordDetailLoading(true);
    setRecordDetailError(null);
    setResponseBody(null);
    setResponseBodyLoading(true);
    setResponseBodyError(null);

    void fetchInvocationRecordDetail(fullRecord.id)
      .then((detail) => {
        if (abnormalSeq !== abnormalSeqRef.current) return;
        setRecordDetail(detail);
      })
      .catch((error) => {
        if (abnormalSeq !== abnormalSeqRef.current) return;
        setRecordDetailError(
          error instanceof Error ? error.message : String(error),
        );
      })
      .finally(() => {
        if (abnormalSeq === abnormalSeqRef.current) {
          setRecordDetailLoading(false);
        }
      });

    void fetchInvocationResponseBody(fullRecord.id)
      .then((detail) => {
        if (abnormalSeq !== abnormalSeqRef.current) return;
        setResponseBody(detail);
      })
      .catch((error) => {
        if (abnormalSeq !== abnormalSeqRef.current) return;
        setResponseBodyError(
          error instanceof Error ? error.message : String(error),
        );
      })
      .finally(() => {
        if (abnormalSeq === abnormalSeqRef.current) {
          setResponseBodyLoading(false);
        }
      });
  }, [fullRecord, open]);

  const recordForHeader = fullRecord ?? selection?.invocation.record ?? null;
  const statusMeta = resolveStatusMeta(
    recordForHeader != null
      ? resolveInvocationDisplayStatus(recordForHeader)
      : selection?.invocation.displayStatus,
  );
  const statusLabel = statusMeta.labelKey
    ? t(statusMeta.labelKey as Parameters<typeof t>[0])
    : (statusMeta.label ?? t("table.status.unknown"));
  const slotLabel =
    selection?.slotKind === "previous"
      ? t("dashboard.workingConversations.previousInvocation")
      : t("dashboard.workingConversations.currentInvocation");
  const occurredAtLabel =
    recordForHeader != null
      ? formatOccurredAtLabel(recordForHeader.occurredAt, dateTimeFormatter)
      : FALLBACK_CELL;
  const poolAttemptsState = useInvocationPoolAttempts(fullRecord);

  const renderAccountValue = useCallback(
    (
      accountLabel: string,
      accountId: number | null,
      accountClickable: boolean,
      className?: string,
    ) => {
      if (!accountClickable || accountId == null) {
        return (
          <span className={className} title={accountLabel}>
            {accountLabel}
          </span>
        );
      }

      return (
        <button
          type="button"
          className={className}
          onClick={() => onOpenUpstreamAccount?.(accountId, accountLabel)}
          title={accountLabel}
        >
          {accountLabel}
        </button>
      );
    },
    [onOpenUpstreamAccount],
  );

  const detailView = useMemo(() => {
    if (!fullRecord) return null;
    return buildInvocationDetailViewModel({
      record: fullRecord,
      normalizedStatus: (
        resolveInvocationDisplayStatus(fullRecord) || "unknown"
      )
        .trim()
        .toLowerCase(),
      t,
      locale,
      localeTag,
      nowMs: Date.now(),
      numberFormatter,
      currencyFormatter,
      renderAccountValue,
    });
  }, [
    currencyFormatter,
    fullRecord,
    locale,
    localeTag,
    numberFormatter,
    renderAccountValue,
    t,
  ]);

  const abnormalResponseBody = responseBody
    ? {
        available: responseBody.available,
        previewText: responseBody.bodyText ?? null,
        hasMore: false,
        unavailableReason: responseBody.unavailableReason ?? null,
      }
    : (recordDetail?.abnormalResponseBody ?? null);
  const displaySequenceId = selection
    ? formatDashboardWorkingConversationSequenceId(
        selection.conversationSequenceId,
      )
    : null;
  const abnormalResponseBodyLoading =
    (recordDetailLoading || responseBodyLoading) &&
    abnormalResponseBody == null;
  const abnormalResponseBodyError =
    abnormalResponseBody != null
      ? null
      : (responseBodyError ?? recordDetailError);

  return (
    <AccountDetailDrawerShell
      open={open}
      labelledBy={titleId}
      closeLabel={t("dashboard.workingConversations.drawer.close")}
      onClose={onClose}
      shellClassName="max-w-[72rem]"
      header={
        <div
          className="space-y-3"
          data-testid="dashboard-invocation-detail-drawer"
        >
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant="secondary">{slotLabel}</Badge>
            <Badge variant={statusMeta.variant}>{statusLabel}</Badge>
            {displaySequenceId ? (
              <Badge variant="secondary">{displaySequenceId}</Badge>
            ) : null}
          </div>
          <div className="section-heading">
            <p className="text-xs font-semibold uppercase tracking-[0.2em] text-primary/75">
              {t("dashboard.workingConversations.drawer.subtitle")}
            </p>
            <h2 id={titleId} className="section-title">
              {t("dashboard.workingConversations.drawer.title")}
            </h2>
            <p className="section-description">{occurredAtLabel}</p>
          </div>
          <div className="space-y-1">
            <p className="break-all font-mono text-sm text-base-content/75">
              {selection?.promptCacheKey ?? FALLBACK_CELL}
            </p>
            <p className="break-all font-mono text-xs text-base-content/58">
              {recordForHeader?.invokeId ?? FALLBACK_CELL}
            </p>
          </div>
        </div>
      }
    >
      {isLoading ? (
        <div
          className="flex min-h-[18rem] items-center justify-center gap-3 rounded-2xl border border-dashed border-base-300/75 bg-base-100/45"
          data-testid="dashboard-invocation-detail-loading"
        >
          <Spinner
            size="sm"
            aria-label={t("dashboard.workingConversations.drawer.loading")}
          />
          <span className="text-sm text-base-content/70">
            {t("dashboard.workingConversations.drawer.loading")}
          </span>
        </div>
      ) : loadError ? (
        <Alert variant="error" data-testid="dashboard-invocation-detail-error">
          <AppIcon
            name="alert-circle-outline"
            className="mt-0.5 h-4 w-4 shrink-0"
            aria-hidden
          />
          <div>
            <p className="font-medium">
              {t("dashboard.workingConversations.drawer.errorTitle")}
            </p>
            <p className="mt-1 text-sm">{loadError}</p>
          </div>
        </Alert>
      ) : !fullRecord || !detailView ? (
        <div
          className="flex min-h-[18rem] flex-col items-center justify-center rounded-[1.6rem] border border-dashed border-base-300/80 bg-base-100/45 px-6 text-center"
          data-testid="dashboard-invocation-detail-empty"
        >
          <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-primary/10 text-primary">
            <AppIcon
              name="account-details-outline"
              className="h-7 w-7"
              aria-hidden
            />
          </div>
          <h3 className="text-lg font-semibold">
            {t("dashboard.workingConversations.drawer.emptyTitle")}
          </h3>
          <p className="mt-2 max-w-sm text-sm leading-6 text-base-content/65">
            {t("dashboard.workingConversations.drawer.emptyBody")}
          </p>
        </div>
      ) : (
        <div className="grid gap-4">
          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
            <SummaryCard label={t("table.details.account")}>
              <div className="text-sm font-medium">
                {renderAccountValue(
                  detailView.accountLabel,
                  detailView.accountId,
                  detailView.accountClickable,
                  "inline-flex max-w-full min-w-0 truncate font-mono text-left text-primary transition hover:opacity-80 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary",
                )}
              </div>
              <div
                className="mt-2 truncate text-xs text-base-content/70"
                title={detailView.proxyDisplayName}
              >
                {detailView.proxyDisplayName}
              </div>
            </SummaryCard>

            <SummaryCard label={t("table.column.model")}>
              <div
                className="flex items-start gap-1 text-sm font-medium"
                title={detailView.modelValue}
              >
                <span className="min-w-0 flex-1 truncate">
                  {detailView.modelValue}
                </span>
              </div>
              <div className="mt-2 w-fit max-w-full">
                {renderEndpointSummary(
                  detailView.endpointDisplay,
                  t,
                  "text-[10px]",
                )}
              </div>
            </SummaryCard>

            <SummaryCard label={t("table.latency.firstByteTotal")}>
              <dl className="grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
                <dt className="text-base-content/60">
                  {t("records.table.network.totalMs")}
                </dt>
                <dd className="truncate text-right font-mono">
                  {detailView.totalLatencyValue}
                </dd>
                <dt className="text-base-content/60">
                  {t("records.table.network.firstResponseByteTotal")}
                </dt>
                <dd className="truncate text-right font-mono">
                  {detailView.firstResponseByteTotalValue}
                </dd>
                <dt className="text-base-content/60">
                  {t("table.details.httpCompression")}
                </dt>
                <dd className="truncate text-right font-mono">
                  {detailView.responseContentEncodingValue}
                </dd>
              </dl>
            </SummaryCard>
          </div>

          <div className="rounded-xl border border-base-300/70 bg-base-200/35 p-4">
            <InvocationExpandedDetails
              record={fullRecord}
              detailId={`dashboard-invocation-details-${fullRecord.id}`}
              detailPairs={detailView.detailPairs}
              timingPairs={detailView.timingPairs}
              errorMessage={detailView.errorMessage}
              detailNotice={detailView.detailNotice}
              size="default"
              poolAttemptsState={poolAttemptsState}
              abnormalResponseBody={abnormalResponseBody}
              abnormalResponseBodyLoading={abnormalResponseBodyLoading}
              abnormalResponseBodyError={abnormalResponseBodyError}
              t={t}
            />
          </div>
        </div>
      )}
    </AccountDetailDrawerShell>
  );
}
