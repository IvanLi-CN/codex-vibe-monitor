import {
  Fragment,
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";
import { AppIcon } from "./AppIcon";
import { AccountDetailDrawerShell } from "./AccountDetailDrawerShell";
import {
  FALLBACK_CELL,
  InvocationExpandedDetails,
  buildInvocationDetailViewModel,
  formatOptionalText,
  renderEndpointSummary,
  renderFastIndicator,
  resolveInvocationCollapsedErrorSummary,
  useInvocationPoolAttempts,
} from "./invocation-details-shared";
import {
  fetchInvocationRecordDetail,
  fetchInvocationResponseBody,
  type ApiInvocation,
  type ApiInvocationRecordDetailResponse,
  type ApiInvocationResponseBodyResponse,
  type InvocationFocus,
} from "../lib/api";
import { invocationStableDomKey, invocationStableKey } from "../lib/invocation";
import { resolveInvocationDisplayStatus } from "../lib/invocationStatus";
import { useTranslation } from "../i18n";
import { Alert } from "./ui/alert";
import { Badge } from "./ui/badge";
import { Spinner } from "./ui/spinner";
import { cn } from "../lib/utils";

interface InvocationRecordsTableProps {
  focus: InvocationFocus;
  records: ApiInvocation[];
  isLoading: boolean;
  error?: string | null;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
}

type StatusMeta = {
  variant: "default" | "secondary" | "success" | "warning" | "error";
  labelKey?: string;
  label?: string;
};

interface InvocationRecordsRowViewModel {
  record: ApiInvocation;
  rowKey: string;
  occurredAtLabel: string;
  statusMeta: StatusMeta;
  statusLabel: string;
  accountLabel: string;
  accountId: number | null;
  accountClickable: boolean;
  proxyDisplayName: string;
  modelValue: string;
  requestedServiceTierValue: string;
  serviceTierValue: string;
  billingServiceTierValue: string;
  fastIndicatorState: ReturnType<
    typeof buildInvocationDetailViewModel
  >["fastIndicatorState"];
  costValue: string;
  inputTokensValue: string;
  cacheInputTokensValue: string;
  outputTokensValue: string;
  outputReasoningBreakdownValue: string;
  reasoningTokensValue: string;
  reasoningEffortValue: string;
  totalTokensValue: string;
  endpointValue: string;
  endpointDisplay: ReturnType<
    typeof buildInvocationDetailViewModel
  >["endpointDisplay"];
  errorMessage: string;
  collapsedErrorSummary: string;
  totalLatencyValue: string;
  firstResponseByteTotalValue: string;
  firstByteLatencyValue: string;
  responseContentEncodingValue: string;
  detailNotice: string | null;
  detailPairs: ReturnType<typeof buildInvocationDetailViewModel>["detailPairs"];
  timingPairs: ReturnType<typeof buildInvocationDetailViewModel>["timingPairs"];
}

const STATUS_META: Record<
  string,
  { variant: StatusMeta["variant"]; labelKey: string }
> = {
  success: { variant: "success", labelKey: "table.status.success" },
  completed: { variant: "success", labelKey: "table.status.success" },
  failed: { variant: "error", labelKey: "table.status.failed" },
  interrupted: { variant: "error", labelKey: "table.status.interrupted" },
  running: { variant: "default", labelKey: "table.status.running" },
  pending: { variant: "warning", labelKey: "table.status.pending" },
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
  const known = STATUS_META[lower];
  if (known) return known;
  if (!raw) return { variant: "secondary", labelKey: "table.status.unknown" };
  if (lower.startsWith("http_4"))
    return { variant: "warning", label: formatStatusLabel(raw) ?? raw };
  if (lower.startsWith("http_5"))
    return { variant: "error", label: formatStatusLabel(raw) ?? raw };
  if (lower.startsWith("http_"))
    return { variant: "secondary", label: formatStatusLabel(raw) ?? raw };
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

function formatOccurredAt(occurredAt: string, formatter: Intl.DateTimeFormat) {
  const value = occurredAt.trim();
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value || FALLBACK_CELL;
  return formatter.format(parsed);
}

function resolveFailureClassMeta(failureClass?: ApiInvocation["failureClass"]) {
  switch (failureClass) {
    case "service_failure":
      return {
        variant: "error" as const,
        labelKey: "records.filters.failureClass.service",
      };
    case "client_failure":
      return {
        variant: "warning" as const,
        labelKey: "records.filters.failureClass.client",
      };
    case "client_abort":
      return {
        variant: "secondary" as const,
        labelKey: "records.filters.failureClass.abort",
      };
    default:
      return { variant: "secondary" as const, labelKey: null };
  }
}

function renderActionableBadge(
  value: ApiInvocation["isActionable"],
  t: ReturnType<typeof useTranslation>["t"],
) {
  if (typeof value !== "boolean") return FALLBACK_CELL;
  return (
    <Badge variant={value ? "warning" : "secondary"}>
      {value
        ? t("records.table.exception.actionableYes")
        : t("records.table.exception.actionableNo")}
    </Badge>
  );
}

function formatActionableText(
  value: ApiInvocation["isActionable"],
  t: ReturnType<typeof useTranslation>["t"],
) {
  if (typeof value !== "boolean") return FALLBACK_CELL;
  return value
    ? t("records.table.exception.actionableYes")
    : t("records.table.exception.actionableNo");
}

function renderFocusSummary(
  row: InvocationRecordsRowViewModel,
  focus: InvocationFocus,
  t: ReturnType<typeof useTranslation>["t"],
) {
  switch (focus) {
    case "network":
      return (
        <dl className="grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
          <dt className="text-base-content/60">
            {t("records.table.network.endpoint")}
          </dt>
          <dd className="flex justify-end">
            {renderEndpointSummary(row.endpointDisplay, t, "text-[10px]")}
          </dd>
          <dt className="text-base-content/60">
            {t("records.table.network.firstResponseByteTotal")}
          </dt>
          <dd className="truncate text-right font-mono">
            {row.firstResponseByteTotalValue}
          </dd>
          <dt className="text-base-content/60">
            {t("records.table.network.totalMs")}
          </dt>
          <dd className="truncate text-right font-mono">
            {row.totalLatencyValue}
          </dd>
          <dt className="text-base-content/60">
            {t("records.table.network.requesterIp")}
          </dt>
          <dd className="truncate text-right font-mono">
            {formatOptionalText(row.record.requesterIp)}
          </dd>
        </dl>
      );
    case "exception": {
      const failureClass = resolveFailureClassMeta(row.record.failureClass);
      return (
        <dl className="grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
          <dt className="text-base-content/60">
            {t("records.table.exception.failureKind")}
          </dt>
          <dd className="truncate text-right font-mono">
            {formatOptionalText(row.record.failureKind)}
          </dd>
          <dt className="text-base-content/60">
            {t("records.table.exception.failureClass")}
          </dt>
          <dd className="flex justify-end">
            <Badge variant={failureClass.variant}>
              {failureClass.labelKey ? t(failureClass.labelKey) : FALLBACK_CELL}
            </Badge>
          </dd>
          <dt className="text-base-content/60">
            {t("records.table.exception.actionable")}
          </dt>
          <dd className="flex justify-end">
            {renderActionableBadge(row.record.isActionable, t)}
          </dd>
          <dt className="text-base-content/60">
            {t("records.table.exception.error")}
          </dt>
          <dd className="truncate text-right font-mono">
            {row.collapsedErrorSummary || FALLBACK_CELL}
          </dd>
        </dl>
      );
    }
    case "token":
    default:
      return (
        <dl className="grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
          <dt className="text-base-content/60">
            {t("records.table.token.inputCache")}
          </dt>
          <dd className="truncate text-right font-mono">{`${row.inputTokensValue} / ${row.cacheInputTokensValue}`}</dd>
          <dt className="text-base-content/60">
            {t("records.table.token.outputReasoning")}
          </dt>
          <dd className="truncate text-right font-mono">{`${row.outputTokensValue} / ${row.reasoningTokensValue}`}</dd>
          <dt className="text-base-content/60">
            {t("records.table.token.totalTokens")}
          </dt>
          <dd className="truncate text-right font-mono">
            {row.totalTokensValue}
          </dd>
          <dt className="text-base-content/60">
            {t("records.table.token.cost")}
          </dt>
          <dd className="truncate text-right font-mono">{row.costValue}</dd>
        </dl>
      );
  }
}

function renderDetailSummaryStrip(
  row: InvocationRecordsRowViewModel,
  focus: InvocationFocus,
  t: ReturnType<typeof useTranslation>["t"],
  renderAccountValue: (
    accountLabel: string,
    accountId: number | null,
    accountClickable: boolean,
    className?: string,
  ) => ReactNode,
) {
  return (
    <div
      className="grid gap-3 md:grid-cols-2 xl:grid-cols-4"
      data-testid="records-detail-summary-strip"
    >
      <div className="rounded-xl border border-base-300/70 bg-base-100/65 p-3">
        <div className="mb-2 text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/60">
          {t("table.column.status")}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant={row.statusMeta.variant}>{row.statusLabel}</Badge>
          <span className="truncate text-xs text-base-content/70">
            {row.occurredAtLabel}
          </span>
        </div>
        <div className="mt-2 text-sm font-medium">
          {renderAccountValue(
            row.accountLabel,
            row.accountId,
            row.accountClickable,
          )}
        </div>
        <div
          className="truncate text-xs text-base-content/70"
          title={row.proxyDisplayName}
        >
          {row.proxyDisplayName}
        </div>
      </div>

      <div className="rounded-xl border border-base-300/70 bg-base-100/65 p-3">
        <div className="mb-2 text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/60">
          {t("table.column.model")}
        </div>
        <div
          className="flex items-start gap-1 text-sm font-medium"
          title={row.modelValue}
        >
          <span className="min-w-0 flex-1 truncate">{row.modelValue}</span>
          {renderFastIndicator(row.fastIndicatorState, t)}
        </div>
        <div className="mt-2 w-fit max-w-full">
          {renderEndpointSummary(row.endpointDisplay, t, "text-[10px]")}
        </div>
        <div
          className="mt-1 truncate font-mono text-xs text-base-content/70"
          title={row.endpointValue}
        >
          {row.endpointValue}
        </div>
      </div>

      <div className="rounded-xl border border-base-300/70 bg-base-100/65 p-3">
        <div className="mb-2 text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/60">
          {t("table.latency.firstByteTotal")}
        </div>
        <dl className="grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
          <dt className="text-base-content/60">
            {t("records.table.network.totalMs")}
          </dt>
          <dd className="truncate text-right font-mono">
            {row.totalLatencyValue}
          </dd>
          <dt className="text-base-content/60">
            {t("records.table.network.firstResponseByteTotal")}
          </dt>
          <dd className="truncate text-right font-mono">
            {row.firstResponseByteTotalValue}
          </dd>
          <dt className="text-base-content/60">
            {t("table.details.httpCompression")}
          </dt>
          <dd className="truncate text-right font-mono">
            {row.responseContentEncodingValue}
          </dd>
        </dl>
      </div>

      <div className="rounded-xl border border-base-300/70 bg-base-100/65 p-3">
        <div className="mb-2 text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/60">
          {t("records.table.focusTitle")}
        </div>
        {renderFocusSummary(row, focus, t)}
      </div>
    </div>
  );
}

export function InvocationRecordsTable({
  focus,
  records,
  isLoading,
  error,
  onOpenUpstreamAccount,
}: InvocationRecordsTableProps) {
  const { t, locale } = useTranslation();
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [drawerRecordId, setDrawerRecordId] = useState<number | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const [detailByRecordId, setDetailByRecordId] = useState<
    Record<number, ApiInvocationRecordDetailResponse | undefined>
  >({});
  const [detailLoadingByRecordId, setDetailLoadingByRecordId] = useState<
    Record<number, boolean | undefined>
  >({});
  const [detailErrorByRecordId, setDetailErrorByRecordId] = useState<
    Record<number, string | null | undefined>
  >({});
  const [responseBodyByRecordId, setResponseBodyByRecordId] = useState<
    Record<number, ApiInvocationResponseBodyResponse | undefined>
  >({});
  const [responseBodyLoadingByRecordId, setResponseBodyLoadingByRecordId] =
    useState<Record<number, boolean | undefined>>({});
  const [responseBodyErrorByRecordId, setResponseBodyErrorByRecordId] =
    useState<Record<number, string | null | undefined>>({});
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag),
    [localeTag],
  );
  const costFormatter = useMemo(
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

  const renderAccountValue = (
    accountLabel: string,
    accountId: number | null,
    accountClickable: boolean,
    className?: string,
  ) => {
    if (!accountClickable || accountId == null) {
      return (
        <span
          className={cn(
            "inline-flex max-w-full min-w-0 items-center justify-center truncate whitespace-nowrap leading-none",
            className,
          )}
          title={accountLabel}
        >
          {accountLabel}
        </span>
      );
    }

    return (
      <button
        type="button"
        className={cn(
          "inline-flex max-w-full min-w-0 items-center justify-center truncate whitespace-nowrap appearance-none border-0 bg-transparent p-0 align-middle font-inherit leading-none text-center text-current no-underline shadow-none transition hover:opacity-80 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary",
          className,
        )}
        onClick={() => {
          onOpenUpstreamAccount?.(accountId, accountLabel);
        }}
        title={accountLabel}
      >
        {accountLabel}
      </button>
    );
  };

  const rows = useMemo<InvocationRecordsRowViewModel[]>(
    () =>
      records.map((record) => {
        const rowKey = invocationStableKey(record);
        const normalizedStatus = (
          resolveInvocationDisplayStatus(record) || "unknown"
        ).toLowerCase();
        const statusMeta = resolveStatusMeta(
          resolveInvocationDisplayStatus(record),
        );
        const detailView = buildInvocationDetailViewModel({
          record,
          normalizedStatus,
          t,
          locale,
          localeTag,
          nowMs,
          numberFormatter,
          currencyFormatter: costFormatter,
          renderAccountValue,
        });

        return {
          record,
          rowKey,
          occurredAtLabel: formatOccurredAt(
            record.occurredAt,
            dateTimeFormatter,
          ),
          statusMeta,
          statusLabel: statusMeta.labelKey
            ? t(statusMeta.labelKey)
            : (statusMeta.label ?? t("table.status.unknown")),
          collapsedErrorSummary: resolveInvocationCollapsedErrorSummary(record),
          ...detailView,
        };
      }),
    [
      records,
      t,
      locale,
      localeTag,
      nowMs,
      numberFormatter,
      costFormatter,
      dateTimeFormatter,
    ],
  );

  const hasInFlightRows = useMemo(
    () =>
      rows.some((row) => {
        const normalizedStatus = (
          resolveInvocationDisplayStatus(row.record) || "unknown"
        ).toLowerCase();
        return normalizedStatus === "running" || normalizedStatus === "pending";
      }),
    [rows],
  );

  useEffect(() => {
    setExpandedId((current) => {
      if (current === null) return current;
      return rows.some((row) => row.rowKey === current) ? current : null;
    });
  }, [rows]);

  useEffect(() => {
    setDrawerRecordId((current) => {
      if (current == null) return current;
      return rows.some((row) => row.record.id === current) ? current : null;
    });
  }, [rows]);

  useEffect(() => {
    if (!hasInFlightRows) return;
    setNowMs(Date.now());
    const id = window.setInterval(() => {
      setNowMs(Date.now());
    }, 1000);
    return () => window.clearInterval(id);
  }, [hasInFlightRows]);

  const expandedRecord = useMemo(
    () => rows.find((row) => row.rowKey === expandedId)?.record ?? null,
    [expandedId, rows],
  );
  const drawerRow = useMemo(
    () => rows.find((row) => row.record.id === drawerRecordId) ?? null,
    [drawerRecordId, rows],
  );
  const poolAttemptsState = useInvocationPoolAttempts(expandedRecord);
  const drawerPoolAttemptsState = useInvocationPoolAttempts(
    drawerRow?.record ?? null,
  );

  const ensureRecordDetail = useCallback(
    async (record: ApiInvocation) => {
      if (!isAbnormalRecord(record)) return;
      if (
        detailByRecordId[record.id] !== undefined ||
        detailLoadingByRecordId[record.id]
      )
        return;

      setDetailLoadingByRecordId((current) => ({
        ...current,
        [record.id]: true,
      }));
      setDetailErrorByRecordId((current) => ({
        ...current,
        [record.id]: null,
      }));

      try {
        const detail = await fetchInvocationRecordDetail(record.id);
        setDetailByRecordId((current) => ({ ...current, [record.id]: detail }));
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        setDetailErrorByRecordId((current) => ({
          ...current,
          [record.id]: message,
        }));
      } finally {
        setDetailLoadingByRecordId((current) => ({
          ...current,
          [record.id]: false,
        }));
      }
    },
    [detailByRecordId, detailLoadingByRecordId],
  );

  const ensureResponseBody = useCallback(
    async (record: ApiInvocation) => {
      if (!isAbnormalRecord(record)) return;
      if (
        responseBodyByRecordId[record.id] !== undefined ||
        responseBodyLoadingByRecordId[record.id]
      )
        return;

      setResponseBodyLoadingByRecordId((current) => ({
        ...current,
        [record.id]: true,
      }));
      setResponseBodyErrorByRecordId((current) => ({
        ...current,
        [record.id]: null,
      }));

      try {
        const responseBody = await fetchInvocationResponseBody(record.id);
        setResponseBodyByRecordId((current) => ({
          ...current,
          [record.id]: responseBody,
        }));
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        setResponseBodyErrorByRecordId((current) => ({
          ...current,
          [record.id]: message,
        }));
      } finally {
        setResponseBodyLoadingByRecordId((current) => ({
          ...current,
          [record.id]: false,
        }));
      }
    },
    [responseBodyByRecordId, responseBodyLoadingByRecordId],
  );

  useEffect(() => {
    if (!expandedRecord || !isAbnormalRecord(expandedRecord)) return;
    void ensureRecordDetail(expandedRecord);
  }, [ensureRecordDetail, expandedRecord]);

  useEffect(() => {
    if (!drawerRow || !isAbnormalRecord(drawerRow.record)) return;
    void ensureRecordDetail(drawerRow.record);
    void ensureResponseBody(drawerRow.record);
  }, [drawerRow, ensureRecordDetail, ensureResponseBody]);

  const hasRecords = rows.length > 0;
  const showBlockingError = Boolean(error) && !hasRecords;
  const showInlineError = Boolean(error) && hasRecords;

  if (showBlockingError) {
    return (
      <Alert variant="error">
        {t("records.table.loadError", { error: error ?? "" })}
      </Alert>
    );
  }

  if (isLoading && !hasRecords) {
    return (
      <div className="flex justify-center py-10">
        <Spinner size="lg" aria-label={t("records.table.loadingAria")} />
      </div>
    );
  }

  if (!hasRecords) {
    return <Alert>{t("records.table.empty")}</Alert>;
  }

  const headers = (() => {
    switch (focus) {
      case "network":
        return [
          t("records.table.network.endpoint"),
          t("records.table.network.requesterIp"),
          t("records.table.network.firstResponseByteTotal"),
          t("records.table.network.totalMs"),
        ];
      case "exception":
        return [
          t("records.table.exception.failureKind"),
          t("records.table.exception.failureClass"),
          t("records.table.exception.actionable"),
          t("records.table.exception.error"),
        ];
      case "token":
      default:
        return [
          t("records.table.token.inputCache"),
          t("records.table.token.outputReasoning"),
          t("records.table.token.totalTokens"),
          t("records.table.token.cost"),
        ];
    }
  })();

  const detailColSpan = headers.length + 5;
  const drawerResponseBody = drawerRow
    ? responseBodyByRecordId[drawerRow.record.id]
    : undefined;
  const drawerResponseBodyError = drawerRow
    ? (responseBodyErrorByRecordId[drawerRow.record.id] ?? null)
    : null;
  const drawerResponseBodyLoading = drawerRow
    ? Boolean(responseBodyLoadingByRecordId[drawerRow.record.id])
    : false;

  const renderFocusCells = (row: InvocationRecordsRowViewModel) => {
    switch (focus) {
      case "network":
        return (
          <>
            <td className="px-3 py-3 align-middle text-left text-xs">
              <div className="w-fit max-w-full">
                {renderEndpointSummary(row.endpointDisplay, t, "text-[10px]")}
              </div>
            </td>
            <td className="px-3 py-3 align-middle text-left font-mono text-xs">
              {formatOptionalText(row.record.requesterIp)}
            </td>
            <td className="px-3 py-3 align-middle text-right font-mono text-xs">
              {row.firstResponseByteTotalValue}
            </td>
            <td className="px-3 py-3 align-middle text-right font-mono text-xs">
              {row.totalLatencyValue}
            </td>
          </>
        );
      case "exception": {
        const failureClass = resolveFailureClassMeta(row.record.failureClass);
        return (
          <>
            <td className="px-3 py-3 align-middle text-left font-mono text-xs">
              {formatOptionalText(row.record.failureKind)}
            </td>
            <td className="px-3 py-3 align-middle text-left text-xs">
              <Badge variant={failureClass.variant}>
                {failureClass.labelKey
                  ? t(failureClass.labelKey)
                  : FALLBACK_CELL}
              </Badge>
            </td>
            <td className="px-3 py-3 align-middle text-left text-xs">
              {renderActionableBadge(row.record.isActionable, t)}
            </td>
            <td
              className="max-w-[18rem] truncate px-3 py-3 align-middle text-left text-xs"
              title={row.collapsedErrorSummary || undefined}
            >
              {row.collapsedErrorSummary || FALLBACK_CELL}
            </td>
          </>
        );
      }
      case "token":
      default:
        return (
          <>
            <td className="px-3 py-3 align-middle text-right font-mono text-xs">
              <div>{row.inputTokensValue}</div>
              <div className="text-base-content/60">
                {row.cacheInputTokensValue}
              </div>
            </td>
            <td className="px-3 py-3 align-middle text-right font-mono text-xs">
              <div>{row.outputTokensValue}</div>
              <div className="text-base-content/60">
                {row.reasoningTokensValue}
              </div>
            </td>
            <td className="px-3 py-3 align-middle text-right font-mono text-xs">
              {row.totalTokensValue}
            </td>
            <td className="px-3 py-3 align-middle text-right font-mono text-xs">
              {row.costValue}
            </td>
          </>
        );
    }
  };

  const renderMobileFocus = (row: InvocationRecordsRowViewModel) => {
    switch (focus) {
      case "network":
        return (
          <>
            <div className="flex items-center justify-between gap-3">
              <dt>{t("records.table.network.endpoint")}</dt>
              <dd className="flex justify-end">
                {renderEndpointSummary(row.endpointDisplay, t, "text-[10px]")}
              </dd>
            </div>
            <div className="flex items-center justify-between gap-3">
              <dt>{t("records.table.network.requesterIp")}</dt>
              <dd className="truncate font-mono">
                {formatOptionalText(row.record.requesterIp)}
              </dd>
            </div>
            <div className="flex items-center justify-between gap-3">
              <dt>{t("records.table.network.firstResponseByteTotal")}</dt>
              <dd className="font-mono">{row.firstResponseByteTotalValue}</dd>
            </div>
            <div className="flex items-center justify-between gap-3">
              <dt>{t("records.table.network.totalMs")}</dt>
              <dd className="font-mono">{row.totalLatencyValue}</dd>
            </div>
          </>
        );
      case "exception": {
        const failureClass = resolveFailureClassMeta(row.record.failureClass);
        return (
          <>
            <div className="flex items-center justify-between gap-3">
              <dt>{t("records.table.exception.failureKind")}</dt>
              <dd className="truncate font-mono">
                {formatOptionalText(row.record.failureKind)}
              </dd>
            </div>
            <div className="flex items-center justify-between gap-3">
              <dt>{t("records.table.exception.failureClass")}</dt>
              <dd className="truncate">
                {failureClass.labelKey
                  ? t(failureClass.labelKey)
                  : FALLBACK_CELL}
              </dd>
            </div>
            <div className="flex items-center justify-between gap-3">
              <dt>{t("records.table.exception.actionable")}</dt>
              <dd>{formatActionableText(row.record.isActionable, t)}</dd>
            </div>
            <div className="flex items-center justify-between gap-3">
              <dt>{t("records.table.exception.error")}</dt>
              <dd className="truncate font-mono">
                {row.collapsedErrorSummary || FALLBACK_CELL}
              </dd>
            </div>
          </>
        );
      }
      case "token":
      default:
        return (
          <>
            <div className="flex items-center justify-between gap-3">
              <dt>{t("records.table.token.inputCache")}</dt>
              <dd className="font-mono">{`${row.inputTokensValue} / ${row.cacheInputTokensValue}`}</dd>
            </div>
            <div className="flex items-center justify-between gap-3">
              <dt>{t("records.table.token.outputReasoning")}</dt>
              <dd className="font-mono">{`${row.outputTokensValue} / ${row.reasoningTokensValue}`}</dd>
            </div>
            <div className="flex items-center justify-between gap-3">
              <dt>{t("records.table.token.totalTokens")}</dt>
              <dd className="font-mono">{row.totalTokensValue}</dd>
            </div>
            <div className="flex items-center justify-between gap-3">
              <dt>{t("records.table.token.cost")}</dt>
              <dd className="font-mono">{row.costValue}</dd>
            </div>
          </>
        );
    }
  };

  return (
    <div className="space-y-3">
      {showInlineError ? (
        <Alert variant="error">
          {t("records.table.loadError", { error: error ?? "" })}
        </Alert>
      ) : null}

      <div className="space-y-3 md:hidden">
        {rows.map((row) => {
          const detailId = `records-list-details-${invocationStableDomKey(row.rowKey)}`;
          const isExpanded = expandedId === row.rowKey;
          const detail = detailByRecordId[row.record.id];
          const detailLoading = Boolean(detailLoadingByRecordId[row.record.id]);
          const detailError = detailErrorByRecordId[row.record.id] ?? null;
          const abnormalResponseBody = detail?.abnormalResponseBody ?? null;

          return (
            <article
              key={row.rowKey}
              className="rounded-xl border border-base-300/70 bg-base-100/45 px-4 py-4"
            >
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="text-sm font-semibold">
                    {row.occurredAtLabel}
                  </div>
                  <div className="mt-1 flex flex-wrap items-center gap-2">
                    <Badge variant={row.statusMeta.variant}>
                      {row.statusLabel}
                    </Badge>
                    <span className="truncate text-xs text-base-content/70">
                      {row.proxyDisplayName}
                    </span>
                  </div>
                </div>
                <button
                  type="button"
                  className="inline-flex h-8 w-8 items-center justify-center rounded-md text-base-content/70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                  onClick={() =>
                    setExpandedId((current) =>
                      current === row.rowKey ? null : row.rowKey,
                    )
                  }
                  aria-expanded={isExpanded}
                  aria-controls={detailId}
                  aria-label={
                    isExpanded
                      ? t("records.table.hideDetails")
                      : t("records.table.showDetails")
                  }
                >
                  <AppIcon
                    name={isExpanded ? "chevron-down" : "chevron-right"}
                    className="h-5 w-5"
                    aria-hidden
                  />
                </button>
              </div>
              <div className="mt-3">
                <div
                  className="flex items-start gap-1 text-sm font-medium"
                  title={row.modelValue}
                >
                  <span className="min-w-0 flex-1 truncate">
                    {row.modelValue}
                  </span>
                  {renderFastIndicator(row.fastIndicatorState, t)}
                </div>
                <div className="mt-1 flex flex-wrap items-center gap-2 text-xs font-mono text-base-content/70">
                  <span
                    title={row.totalLatencyValue}
                  >{`${t("table.column.totalLatencyShort")} ${row.totalLatencyValue}`}</span>
                  <span title={row.firstResponseByteTotalValue}>
                    {`${t("table.column.firstResponseByteTotalShort")} ${row.firstResponseByteTotalValue}`}
                  </span>
                  <span
                    title={row.responseContentEncodingValue}
                  >{`${t("table.column.httpCompressionShort")} ${row.responseContentEncodingValue}`}</span>
                </div>
              </div>

              <dl className="mt-3 space-y-2 text-xs text-base-content/75">
                {renderMobileFocus(row)}
              </dl>

              {isExpanded ? (
                <div className="mt-3 space-y-3 rounded-xl border border-base-300/70 bg-base-200/55 p-3">
                  {renderDetailSummaryStrip(row, focus, t, renderAccountValue)}
                  <InvocationExpandedDetails
                    record={row.record}
                    detailId={detailId}
                    detailPairs={row.detailPairs}
                    timingPairs={row.timingPairs}
                    errorMessage={row.errorMessage}
                    detailNotice={row.detailNotice}
                    size="compact"
                    poolAttemptsState={poolAttemptsState}
                    abnormalResponseBody={abnormalResponseBody}
                    abnormalResponseBodyLoading={detailLoading}
                    abnormalResponseBodyError={detailError}
                    onOpenFullDetails={
                      isAbnormalRecord(row.record)
                        ? () => setDrawerRecordId(row.record.id)
                        : null
                    }
                    showFullDetailsAction={isAbnormalRecord(row.record)}
                    t={t}
                  />
                </div>
              ) : null}
            </article>
          );
        })}
      </div>

      <div className="hidden overflow-x-auto rounded-xl border border-base-300/70 bg-base-100/50 md:block">
        <table className="min-w-full table-fixed border-separate border-spacing-0 text-sm">
          <thead className="bg-base-200/65 text-[11px] uppercase tracking-[0.08em] text-base-content/70">
            <tr>
              <th className="px-3 py-3 text-left font-semibold">
                {t("table.column.time")}
              </th>
              <th className="px-3 py-3 text-left font-semibold">
                {t("table.column.proxy")}
              </th>
              <th className="px-3 py-3 text-left font-semibold">
                {t("table.column.model")}
              </th>
              <th className="px-3 py-3 text-left font-semibold">
                {t("table.column.status")}
              </th>
              {headers.map((header) => (
                <th key={header} className="px-3 py-3 text-left font-semibold">
                  {header}
                </th>
              ))}
              <th className="px-3 py-3 text-right font-semibold">
                <span className="sr-only">{t("records.table.details")}</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((row, index) => {
              const detailId = `records-table-details-${invocationStableDomKey(row.rowKey)}`;
              const isExpanded = expandedId === row.rowKey;
              const detail = detailByRecordId[row.record.id];
              const detailLoading = Boolean(
                detailLoadingByRecordId[row.record.id],
              );
              const detailError = detailErrorByRecordId[row.record.id] ?? null;
              const abnormalResponseBody = detail?.abnormalResponseBody ?? null;

              return (
                <Fragment key={row.rowKey}>
                  <tr
                    className={
                      index % 2 === 0 ? "bg-base-100/30" : "bg-base-200/18"
                    }
                  >
                    <td className="px-3 py-3 align-middle text-left text-xs font-medium">
                      {row.occurredAtLabel}
                    </td>
                    <td
                      className="max-w-[12rem] truncate px-3 py-3 align-middle text-left text-xs"
                      title={row.proxyDisplayName}
                    >
                      {row.proxyDisplayName}
                    </td>
                    <td className="max-w-[14rem] px-3 py-3 align-middle text-left text-xs">
                      <div
                        className="flex items-start gap-1"
                        title={row.modelValue}
                      >
                        <span className="min-w-0 flex-1 truncate">
                          {row.modelValue}
                        </span>
                        {renderFastIndicator(row.fastIndicatorState, t)}
                      </div>
                    </td>
                    <td className="px-3 py-3 align-middle text-left text-xs">
                      <Badge variant={row.statusMeta.variant}>
                        {row.statusLabel}
                      </Badge>
                    </td>
                    {renderFocusCells(row)}
                    <td className="px-3 py-3 align-middle text-right">
                      <button
                        type="button"
                        className="inline-flex items-center justify-center rounded-md text-base-content/70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                        onClick={() =>
                          setExpandedId((current) =>
                            current === row.rowKey ? null : row.rowKey,
                          )
                        }
                        aria-expanded={isExpanded}
                        aria-controls={detailId}
                        aria-label={
                          isExpanded
                            ? t("records.table.hideDetails")
                            : t("records.table.showDetails")
                        }
                      >
                        <AppIcon
                          name={isExpanded ? "chevron-down" : "chevron-right"}
                          className="h-4 w-4"
                          aria-hidden
                        />
                      </button>
                    </td>
                  </tr>
                  {isExpanded ? (
                    <tr className="bg-base-200/55">
                      <td colSpan={detailColSpan} className="px-4 py-4">
                        <div className="space-y-3 rounded-xl border border-base-300/70 bg-base-200/45 p-3">
                          {renderDetailSummaryStrip(
                            row,
                            focus,
                            t,
                            renderAccountValue,
                          )}
                          <InvocationExpandedDetails
                            record={row.record}
                            detailId={detailId}
                            detailPairs={row.detailPairs}
                            timingPairs={row.timingPairs}
                            errorMessage={row.errorMessage}
                            detailNotice={row.detailNotice}
                            size="default"
                            poolAttemptsState={poolAttemptsState}
                            abnormalResponseBody={abnormalResponseBody}
                            abnormalResponseBodyLoading={detailLoading}
                            abnormalResponseBodyError={detailError}
                            onOpenFullDetails={
                              isAbnormalRecord(row.record)
                                ? () => setDrawerRecordId(row.record.id)
                                : null
                            }
                            showFullDetailsAction={isAbnormalRecord(row.record)}
                            t={t}
                          />
                        </div>
                      </td>
                    </tr>
                  ) : null}
                </Fragment>
              );
            })}
          </tbody>
        </table>
      </div>

      {drawerRow ? (
        <AccountDetailDrawerShell
          open
          labelledBy={`records-full-detail-title-${drawerRow.record.id}`}
          closeLabel={t("records.table.fullDetails.close")}
          onClose={() => setDrawerRecordId(null)}
          shellClassName="max-w-4xl border-l border-base-300 bg-base-100 shadow-2xl"
          bodyClassName="space-y-4"
          header={
            <div className="space-y-2">
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant={drawerRow.statusMeta.variant}>
                  {drawerRow.statusLabel}
                </Badge>
                <span className="text-xs text-base-content/60">
                  {drawerRow.occurredAtLabel}
                </span>
              </div>
              <div>
                <h2
                  id={`records-full-detail-title-${drawerRow.record.id}`}
                  className="text-lg font-semibold"
                >
                  {t("records.table.fullDetails.title")}
                </h2>
                <p className="mt-1 break-all font-mono text-sm text-base-content/70">
                  {drawerRow.record.invokeId}
                </p>
              </div>
            </div>
          }
        >
          {renderDetailSummaryStrip(drawerRow, focus, t, renderAccountValue)}

          <div className="rounded-xl border border-base-300/70 bg-base-200/35 p-4">
            <InvocationExpandedDetails
              record={drawerRow.record}
              detailId={`records-drawer-details-${drawerRow.record.id}`}
              detailPairs={drawerRow.detailPairs}
              timingPairs={drawerRow.timingPairs}
              errorMessage={drawerRow.errorMessage}
              detailNotice={drawerRow.detailNotice}
              size="default"
              poolAttemptsState={drawerPoolAttemptsState}
              abnormalResponseBody={
                drawerResponseBody
                  ? {
                      available: drawerResponseBody.available,
                      previewText: drawerResponseBody.bodyText ?? null,
                      hasMore: false,
                      unavailableReason:
                        drawerResponseBody.unavailableReason ?? null,
                    }
                  : null
              }
              abnormalResponseBodyLoading={drawerResponseBodyLoading}
              abnormalResponseBodyError={drawerResponseBodyError}
              t={t}
            />
          </div>
        </AccountDetailDrawerShell>
      ) : null}
    </div>
  );
}
