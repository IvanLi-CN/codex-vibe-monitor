import { Fragment, type ReactNode, useCallback, useEffect, useMemo, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { Badge } from "../../components/ui/badge";
import { useTranslation } from "../../i18n";
import type { ApiInvocation, InvocationFocus } from "../../lib/api";
import { invocationStableDomKey, invocationStableKey } from "../../lib/invocation";
import { resolveInvocationDisplayStatus } from "../../lib/invocationStatus";
import { cn } from "../../lib/utils";
import { AccountDetailDrawerShell } from "../account-pool/AccountDetailDrawerShell";
import { InvocationWorkflowDetailPanel } from "../invocations/InvocationWorkflowDetailPanel";
import {
  renderInvocationCostAuditWarning,
  resolveInvocationCostAuditDisplay,
} from "../invocations/invocation-cost-audit";
import {
  buildInvocationDetailViewModel,
  FALLBACK_CELL,
  formatOptionalText,
  INVOCATION_ACCOUNT_ROUTING_IN_PROGRESS_CLASS_NAME,
  renderEndpointSummary,
  renderFastIndicator,
  renderImageIntentBadge,
  renderInvocationModelBadge,
  renderInvocationModelRoutingSummary,
  renderReasoningEffortBadge,
} from "../invocations/invocation-details-shared";
import { renderInvocationTransportBadge } from "../invocations/invocation-transport-badge";
import { AppIcon } from "../shared/AppIcon";
import { ListBodyState } from "../shared/ListBodyState";

interface InvocationRecordsTableProps {
  focus: InvocationFocus;
  records: ApiInvocation[];
  isLoading: boolean;
  error?: string | null;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  autoExpandInvokeId?: string | null;
  focusedAttemptId?: string | null;
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
  accountRoutingInProgress: boolean;
  proxyDisplayName: string;
  modelValue: string;
  modelHasMismatch: boolean;
  requestModelValue: string;
  responseModelValue: string;
  requestedServiceTierValue: string;
  serviceTierValue: string;
  billingServiceTierValue: string;
  fastIndicatorState: ReturnType<typeof buildInvocationDetailViewModel>["fastIndicatorState"];
  costValue: string;
  inputTokensValue: string;
  cacheWriteTokensValue: string;
  cacheInputTokensValue: string;
  outputTokensValue: string;
  outputReasoningBreakdownValue: string;
  reasoningTokensValue: string;
  reasoningEffortValue: string;
  totalTokensValue: string;
  endpointValue: string;
  endpointDisplay: ReturnType<typeof buildInvocationDetailViewModel>["endpointDisplay"];
  imageIntentDisplay: ReturnType<typeof buildInvocationDetailViewModel>["imageIntentDisplay"];
  errorMessage: string;
  collapsedErrorSummary: string;
  totalLatencyValue: string;
  firstResponseByteTotalValue: string;
  firstByteLatencyValue: string;
  responseContentEncodingValue: string;
  localCostValue: string;
  costMismatch: boolean;
  costAuditReason: string | null;
  detailNotice: string | null;
  detailPairs: ReturnType<typeof buildInvocationDetailViewModel>["detailPairs"];
  timingPairs: ReturnType<typeof buildInvocationDetailViewModel>["timingPairs"];
}

const STATUS_META: Record<string, { variant: StatusMeta["variant"]; labelKey: string }> = {
  success: { variant: "success", labelKey: "table.status.success" },
  completed: { variant: "success", labelKey: "table.status.success" },
  warning_success: { variant: "warning", labelKey: "table.status.warningSuccess" },
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
  if (lower.startsWith("http_5")) return { variant: "error", label: formatStatusLabel(raw) ?? raw };
  if (lower.startsWith("http_"))
    return { variant: "secondary", label: formatStatusLabel(raw) ?? raw };
  return { variant: "secondary", label: raw };
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

function formatOptionalCurrency(
  value: number | null | undefined,
  formatter: Intl.NumberFormat,
): string {
  return typeof value === "number" && Number.isFinite(value)
    ? formatter.format(value)
    : FALLBACK_CELL;
}

function renderCostAuditSummary(
  row: InvocationRecordsRowViewModel,
  t: ReturnType<typeof useTranslation>["t"],
  costFormatter: Intl.NumberFormat,
  testId?: string,
  showWarning = true,
) {
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-end gap-1">
        <span className="font-mono text-base-content/84">{row.costValue}</span>
        {showWarning
          ? renderInvocationCostAuditWarning(
              row.record.costAudit,
              t,
              (value) => formatOptionalCurrency(value, costFormatter),
              { testId },
            )
          : null}
      </div>
      <div className="text-[11px] font-mono text-base-content/65">
        {`${t("records.costAudit.localShort")} ${row.localCostValue}`}
      </div>
      {!row.costMismatch && row.costAuditReason ? (
        <div className="text-[11px] text-base-content/52">
          {t("records.costAudit.notComparable")}
        </div>
      ) : null}
    </div>
  );
}

function renderFocusSummary(
  row: InvocationRecordsRowViewModel,
  focus: InvocationFocus,
  t: ReturnType<typeof useTranslation>["t"],
  costFormatter: Intl.NumberFormat,
  showCostWarning = true,
) {
  switch (focus) {
    case "network":
      return (
        <dl className="grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
          <dt className="text-base-content/60">{t("records.table.network.endpoint")}</dt>
          <dd className="flex justify-end">
            {renderEndpointSummary(row.endpointDisplay, t, "text-[10px]")}
          </dd>
          <dt className="text-base-content/60">
            {t("records.table.network.firstResponseByteTotal")}
          </dt>
          <dd className="truncate text-right font-mono">{row.firstResponseByteTotalValue}</dd>
          <dt className="text-base-content/60">{t("records.table.network.totalMs")}</dt>
          <dd className="truncate text-right font-mono">{row.totalLatencyValue}</dd>
          <dt className="text-base-content/60">{t("records.table.network.requesterIp")}</dt>
          <dd className="truncate text-right font-mono">
            {formatOptionalText(row.record.requesterIp)}
          </dd>
        </dl>
      );
    case "exception": {
      const failureClass = resolveFailureClassMeta(row.record.failureClass);
      return (
        <dl className="grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
          <dt className="text-base-content/60">{t("records.table.exception.failureKind")}</dt>
          <dd className="truncate text-right font-mono">
            {formatOptionalText(row.record.failureKind)}
          </dd>
          <dt className="text-base-content/60">{t("records.table.exception.failureClass")}</dt>
          <dd className="flex justify-end">
            <Badge variant={failureClass.variant}>
              {failureClass.labelKey ? t(failureClass.labelKey) : FALLBACK_CELL}
            </Badge>
          </dd>
          <dt className="text-base-content/60">{t("records.table.exception.actionable")}</dt>
          <dd className="flex justify-end">{renderActionableBadge(row.record.isActionable, t)}</dd>
          <dt className="text-base-content/60">{t("records.table.exception.error")}</dt>
          <dd className="truncate text-right font-mono">
            {row.collapsedErrorSummary || FALLBACK_CELL}
          </dd>
        </dl>
      );
    }
    default:
      return (
        <dl className="grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
          <dt className="text-base-content/60">{t("records.table.token.inputCache")}</dt>
          <dd className="truncate text-right font-mono">{`IN ${row.inputTokensValue} / CW ${row.cacheWriteTokensValue} / C ${row.cacheInputTokensValue}`}</dd>
          <dt className="text-base-content/60">{t("records.table.token.outputReasoning")}</dt>
          <dd className="truncate text-right font-mono">{`${row.outputTokensValue} / ${row.reasoningTokensValue}`}</dd>
          <dt className="text-base-content/60">{t("records.table.token.totalTokens")}</dt>
          <dd className="truncate text-right font-mono">{row.totalTokensValue}</dd>
          <dt className="text-base-content/60">{t("records.table.token.cost")}</dt>
          <dd className="text-right">
            {renderCostAuditSummary(row, t, costFormatter, undefined, showCostWarning)}
          </dd>
        </dl>
      );
  }
}

function renderDetailSummaryStrip(
  row: InvocationRecordsRowViewModel,
  focus: InvocationFocus,
  t: ReturnType<typeof useTranslation>["t"],
  costFormatter: Intl.NumberFormat,
  renderAccountValue: (
    accountLabel: string,
    accountId: number | null,
    accountClickable: boolean,
    className?: string,
  ) => ReactNode,
  layout: "grid" | "flat" = "grid",
) {
  if (layout === "flat") {
    return (
      <div className="space-y-4" data-testid="records-detail-summary-strip">
        <section className="space-y-2">
          <div className="text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/60">
            {t("table.column.status")}
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant={row.statusMeta.variant}>{row.statusLabel}</Badge>
            <span className="truncate text-xs text-base-content/70">{row.occurredAtLabel}</span>
          </div>
          <div className="text-sm font-medium">
            {renderAccountValue(
              row.accountLabel,
              row.accountId,
              row.accountClickable,
              row.accountRoutingInProgress
                ? INVOCATION_ACCOUNT_ROUTING_IN_PROGRESS_CLASS_NAME
                : undefined,
            )}
          </div>
          <div className="truncate text-xs text-base-content/70" title={row.proxyDisplayName}>
            {row.proxyDisplayName}
          </div>
        </section>

        <section className="space-y-2 border-t border-base-300/60 pt-4">
          <div className="text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/60">
            {t("table.column.model")}
          </div>
          {row.modelHasMismatch ? (
            renderInvocationModelRoutingSummary({
              requestModelValue: row.requestModelValue,
              responseModelValue: row.responseModelValue,
              hasMismatch: true,
              t,
              indicatorTestId: "invocation-records-model-routing-indicator",
              adornments: (
                <>
                  {renderReasoningEffortBadge(row.reasoningEffortValue)}
                  {renderImageIntentBadge(row.imageIntentDisplay, t)}
                  {renderInvocationTransportBadge(row.record)}
                  {renderFastIndicator(row.fastIndicatorState, t)}
                </>
              ),
            })
          ) : (
            <div
              className="flex min-w-0 flex-wrap items-center gap-1 text-sm font-medium"
              title={row.modelValue}
            >
              {renderInvocationModelBadge(row.modelValue, {
                t,
                hasMismatch: false,
                testId: "invocation-records-model",
              })}
              {renderReasoningEffortBadge(row.reasoningEffortValue)}
              {renderImageIntentBadge(row.imageIntentDisplay, t)}
              {renderInvocationTransportBadge(row.record)}
              {renderFastIndicator(row.fastIndicatorState, t)}
            </div>
          )}
        </section>

        <section className="space-y-2 border-t border-base-300/60 pt-4">
          <div className="text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/60">
            {t("table.latency.firstByteTotal")}
          </div>
          <dl className="space-y-2 text-xs">
            <div className="grid min-w-0 grid-cols-[4.75rem_minmax(0,1fr)] items-start gap-3">
              <dt className="text-base-content/60">{t("records.table.network.totalMs")}</dt>
              <dd className="min-w-0 text-right font-mono">{row.totalLatencyValue}</dd>
            </div>
            <div className="grid min-w-0 grid-cols-[4.75rem_minmax(0,1fr)] items-start gap-3">
              <dt className="text-base-content/60">
                {t("records.table.network.firstResponseByteTotal")}
              </dt>
              <dd className="min-w-0 text-right font-mono">{row.firstResponseByteTotalValue}</dd>
            </div>
            <div className="grid min-w-0 grid-cols-[4.75rem_minmax(0,1fr)] items-start gap-3">
              <dt className="text-base-content/60">{t("table.details.httpCompression")}</dt>
              <dd className="min-w-0 break-all text-right font-mono">
                {row.responseContentEncodingValue}
              </dd>
            </div>
          </dl>
        </section>

        <section className="space-y-2 border-t border-base-300/60 pt-4">
          <div className="flex items-center justify-between gap-2 text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/60">
            {t("records.table.focusTitle")}
            {renderInvocationCostAuditWarning(
              row.record.costAudit,
              t,
              (value) => formatOptionalCurrency(value, costFormatter),
              { testId: "records-detail-strip-cost-warning" },
            )}
          </div>
          {renderFocusSummary(row, focus, t, costFormatter, false)}
        </section>
      </div>
    );
  }

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
          <span className="truncate text-xs text-base-content/70">{row.occurredAtLabel}</span>
        </div>
        <div className="mt-2 text-sm font-medium">
          {renderAccountValue(
            row.accountLabel,
            row.accountId,
            row.accountClickable,
            row.accountRoutingInProgress
              ? INVOCATION_ACCOUNT_ROUTING_IN_PROGRESS_CLASS_NAME
              : undefined,
          )}
        </div>
        <div className="truncate text-xs text-base-content/70" title={row.proxyDisplayName}>
          {row.proxyDisplayName}
        </div>
      </div>

      <div className="rounded-xl border border-base-300/70 bg-base-100/65 p-3">
        <div className="mb-2 text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/60">
          {t("table.column.model")}
        </div>
        {row.modelHasMismatch ? (
          renderInvocationModelRoutingSummary({
            requestModelValue: row.requestModelValue,
            responseModelValue: row.responseModelValue,
            hasMismatch: true,
            t,
            indicatorTestId: "invocation-records-model-routing-indicator",
            adornments: (
              <>
                {renderReasoningEffortBadge(row.reasoningEffortValue)}
                {renderImageIntentBadge(row.imageIntentDisplay, t)}
                {renderInvocationTransportBadge(row.record)}
                {renderFastIndicator(row.fastIndicatorState, t)}
              </>
            ),
          })
        ) : (
          <div
            className="flex min-w-0 flex-wrap items-center gap-1 text-sm font-medium"
            title={row.modelValue}
          >
            {renderInvocationModelBadge(row.modelValue, {
              t,
              hasMismatch: false,
              testId: "invocation-records-model",
            })}
            {renderReasoningEffortBadge(row.reasoningEffortValue)}
            {renderImageIntentBadge(row.imageIntentDisplay, t)}
            {renderInvocationTransportBadge(row.record)}
            {renderFastIndicator(row.fastIndicatorState, t)}
          </div>
        )}
      </div>

      <div className="rounded-xl border border-base-300/70 bg-base-100/65 p-3">
        <div className="mb-2 text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/60">
          {t("table.latency.firstByteTotal")}
        </div>
        <dl className="grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
          <dt className="text-base-content/60">{t("records.table.network.totalMs")}</dt>
          <dd className="truncate text-right font-mono">{row.totalLatencyValue}</dd>
          <dt className="text-base-content/60">
            {t("records.table.network.firstResponseByteTotal")}
          </dt>
          <dd className="truncate text-right font-mono">{row.firstResponseByteTotalValue}</dd>
          <dt className="text-base-content/60">{t("table.details.httpCompression")}</dt>
          <dd className="truncate text-right font-mono">{row.responseContentEncodingValue}</dd>
        </dl>
      </div>

      <div className="rounded-xl border border-base-300/70 bg-base-100/65 p-3">
        <div className="mb-2 flex items-center justify-between gap-2 text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/60">
          {t("records.table.focusTitle")}
          {renderInvocationCostAuditWarning(
            row.record.costAudit,
            t,
            (value) => formatOptionalCurrency(value, costFormatter),
            { testId: "records-detail-strip-cost-warning" },
          )}
        </div>
        {renderFocusSummary(row, focus, t, costFormatter, false)}
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
  autoExpandInvokeId = null,
  focusedAttemptId = null,
}: InvocationRecordsTableProps) {
  const { t, locale } = useTranslation();
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [drawerRecordId, setDrawerRecordId] = useState<number | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag), [localeTag]);
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

  const renderAccountValue = useCallback(
    (
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
    },
    [onOpenUpstreamAccount],
  );

  const rows = useMemo<InvocationRecordsRowViewModel[]>(
    () =>
      records.map((record) => {
        const rowKey = invocationStableKey(record);
        const normalizedStatus = (
          resolveInvocationDisplayStatus(record) || "unknown"
        ).toLowerCase();
        const statusMeta = resolveStatusMeta(resolveInvocationDisplayStatus(record));
        const costAuditDisplay = resolveInvocationCostAuditDisplay(
          record.costAudit,
          record.cost ?? null,
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
          occurredAtLabel: formatOccurredAt(record.occurredAt, dateTimeFormatter),
          statusMeta,
          statusLabel: statusMeta.labelKey
            ? t(statusMeta.labelKey)
            : (statusMeta.label ?? t("table.status.unknown")),
          localCostValue: formatOptionalCurrency(costAuditDisplay.localTotal, costFormatter),
          costMismatch: costAuditDisplay.mismatch,
          costAuditReason: costAuditDisplay.reason,
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
      renderAccountValue,
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
    if (!autoExpandInvokeId) return;
    const row = rows.find((candidate) => candidate.record.invokeId === autoExpandInvokeId);
    if (row) setExpandedId(row.rowKey);
  }, [autoExpandInvokeId, rows]);

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

  const drawerRow = useMemo(
    () => rows.find((row) => row.record.id === drawerRecordId) ?? null,
    [drawerRecordId, rows],
  );

  const hasRecords = rows.length > 0;
  const showBlockingError = Boolean(error) && !hasRecords;
  const showInlineError = Boolean(error) && hasRecords;

  if (showBlockingError) {
    return (
      <ListBodyState
        variant="error"
        title={t("records.table.loadError", { error: error ?? "" })}
        testId="invocation-records-table-error"
      />
    );
  }

  if (isLoading && !hasRecords) {
    return (
      <ListBodyState
        variant="loading"
        title={t("records.table.loadingAria")}
        testId="invocation-records-table-loading"
      />
    );
  }

  if (!hasRecords) {
    return (
      <ListBodyState
        variant="empty"
        title={t("records.table.empty")}
        testId="invocation-records-table-empty"
      />
    );
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

  const renderFocusCells = (row: InvocationRecordsRowViewModel, isExpanded: boolean) => {
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
                {failureClass.labelKey ? t(failureClass.labelKey) : FALLBACK_CELL}
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
      default:
        return (
          <>
            <td className="px-3 py-3 align-middle text-right font-mono text-xs">
              <div role="img" aria-label={`Input tokens: ${row.inputTokensValue}`}>
                IN {row.inputTokensValue}
              </div>
              <div
                role="img"
                title="Cache write tokens"
                aria-label={`Cache write tokens: ${row.cacheWriteTokensValue}`}
              >
                CW {row.cacheWriteTokensValue}
              </div>
              <div
                role="img"
                className="text-base-content/60"
                title="Cache read tokens"
                aria-label={`Cache read tokens: ${row.cacheInputTokensValue}`}
              >
                C {row.cacheInputTokensValue}
              </div>
            </td>
            <td className="px-3 py-3 align-middle text-right font-mono text-xs">
              <div>{row.outputTokensValue}</div>
              <div className="text-base-content/60">{row.reasoningTokensValue}</div>
            </td>
            <td className="px-3 py-3 align-middle text-right font-mono text-xs">
              {row.totalTokensValue}
            </td>
            <td className="px-3 py-3 align-middle text-right text-xs">
              {renderCostAuditSummary(
                row,
                t,
                costFormatter,
                "records-table-cost-warning",
                !isExpanded,
              )}
            </td>
          </>
        );
    }
  };

  const renderMobileFocus = (row: InvocationRecordsRowViewModel, isExpanded: boolean) => {
    switch (focus) {
      case "network":
        return (
          <>
            <div className="grid min-w-0 grid-cols-[4rem_minmax(0,1fr)] items-start gap-3">
              <dt>{t("records.table.network.endpoint")}</dt>
              <dd className="flex min-w-0 justify-end">
                {renderEndpointSummary(row.endpointDisplay, t, "text-[10px]")}
              </dd>
            </div>
            <div className="grid min-w-0 grid-cols-[4rem_minmax(0,1fr)] items-start gap-3">
              <dt>{t("records.table.network.requesterIp")}</dt>
              <dd className="min-w-0 break-all text-right font-mono">
                {formatOptionalText(row.record.requesterIp)}
              </dd>
            </div>
            <div className="grid min-w-0 grid-cols-[4rem_minmax(0,1fr)] items-start gap-3">
              <dt>{t("records.table.network.firstResponseByteTotal")}</dt>
              <dd className="min-w-0 text-right font-mono">{row.firstResponseByteTotalValue}</dd>
            </div>
            <div className="grid min-w-0 grid-cols-[4rem_minmax(0,1fr)] items-start gap-3">
              <dt>{t("records.table.network.totalMs")}</dt>
              <dd className="min-w-0 text-right font-mono">{row.totalLatencyValue}</dd>
            </div>
          </>
        );
      case "exception": {
        const failureClass = resolveFailureClassMeta(row.record.failureClass);
        return (
          <>
            <div className="grid min-w-0 grid-cols-[4.75rem_minmax(0,1fr)] items-start gap-3">
              <dt>{t("records.table.exception.failureKind")}</dt>
              <dd className="min-w-0 break-all text-right font-mono">
                {formatOptionalText(row.record.failureKind)}
              </dd>
            </div>
            <div className="grid min-w-0 grid-cols-[4.75rem_minmax(0,1fr)] items-start gap-3">
              <dt>{t("records.table.exception.failureClass")}</dt>
              <dd className="min-w-0 text-right">
                {failureClass.labelKey ? t(failureClass.labelKey) : FALLBACK_CELL}
              </dd>
            </div>
            <div className="grid min-w-0 grid-cols-[4.75rem_minmax(0,1fr)] items-start gap-3">
              <dt>{t("records.table.exception.actionable")}</dt>
              <dd className="min-w-0 text-right">
                {formatActionableText(row.record.isActionable, t)}
              </dd>
            </div>
            <div className="grid min-w-0 grid-cols-[4.75rem_minmax(0,1fr)] items-start gap-3">
              <dt>{t("records.table.exception.error")}</dt>
              <dd className="min-w-0 break-all text-right font-mono">
                {row.collapsedErrorSummary || FALLBACK_CELL}
              </dd>
            </div>
          </>
        );
      }
      default:
        return (
          <>
            <div className="grid min-w-0 grid-cols-[4.75rem_minmax(0,1fr)] items-start gap-3">
              <dt>{t("records.table.token.inputCache")}</dt>
              <dd className="min-w-0 break-all text-right font-mono">{`IN ${row.inputTokensValue} / CW ${row.cacheWriteTokensValue} / C ${row.cacheInputTokensValue}`}</dd>
            </div>
            <div className="grid min-w-0 grid-cols-[4.75rem_minmax(0,1fr)] items-start gap-3">
              <dt>{t("records.table.token.outputReasoning")}</dt>
              <dd className="min-w-0 text-right font-mono">{`${row.outputTokensValue} / ${row.reasoningTokensValue}`}</dd>
            </div>
            <div className="grid min-w-0 grid-cols-[4.75rem_minmax(0,1fr)] items-start gap-3">
              <dt>{t("records.table.token.totalTokens")}</dt>
              <dd className="min-w-0 text-right font-mono">{row.totalTokensValue}</dd>
            </div>
            <div className="grid min-w-0 grid-cols-[4.75rem_minmax(0,1fr)] items-start gap-3">
              <dt>{t("records.table.token.cost")}</dt>
              <dd className="min-w-0 text-right">
                {renderCostAuditSummary(
                  row,
                  t,
                  costFormatter,
                  "records-mobile-cost-warning",
                  !isExpanded,
                )}
              </dd>
            </div>
          </>
        );
    }
  };

  return (
    <div className="min-w-0 max-w-full space-y-3">
      {showInlineError ? (
        <Alert variant="error">{t("records.table.loadError", { error: error ?? "" })}</Alert>
      ) : null}

      <div className="space-y-3 md:hidden">
        {rows.map((row) => {
          const detailId = `records-list-details-${invocationStableDomKey(row.rowKey)}`;
          const isExpanded = expandedId === row.rowKey;

          return (
            <article
              key={row.rowKey}
              className="min-w-0 overflow-hidden rounded-xl border border-base-300/70 bg-base-100/45 px-3.5 py-3.5"
            >
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="text-sm font-semibold">{row.occurredAtLabel}</div>
                  <div className="mt-1 flex flex-wrap items-center gap-2">
                    <Badge variant={row.statusMeta.variant}>{row.statusLabel}</Badge>
                    <span className="truncate text-xs text-base-content/70">
                      {row.proxyDisplayName}
                    </span>
                  </div>
                </div>
                <button
                  type="button"
                  className="inline-flex h-8 w-8 items-center justify-center rounded-md text-base-content/70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                  onClick={() =>
                    setExpandedId((current) => (current === row.rowKey ? null : row.rowKey))
                  }
                  aria-expanded={isExpanded}
                  aria-controls={detailId}
                  aria-label={
                    isExpanded ? t("records.table.hideDetails") : t("records.table.showDetails")
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
                {row.modelHasMismatch ? (
                  renderInvocationModelRoutingSummary({
                    requestModelValue: row.requestModelValue,
                    responseModelValue: row.responseModelValue,
                    hasMismatch: true,
                    t,
                    indicatorTestId: "invocation-records-model-routing-indicator",
                    adornments: (
                      <>
                        {renderReasoningEffortBadge(row.reasoningEffortValue)}
                        {renderImageIntentBadge(row.imageIntentDisplay, t)}
                        {renderInvocationTransportBadge(row.record)}
                        {renderFastIndicator(row.fastIndicatorState, t)}
                      </>
                    ),
                  })
                ) : (
                  <div
                    className="flex min-w-0 flex-wrap items-center gap-1 text-sm font-medium"
                    title={row.modelValue}
                  >
                    {renderInvocationModelBadge(row.modelValue, {
                      t,
                      hasMismatch: false,
                      testId: "invocation-records-model",
                    })}
                    {renderReasoningEffortBadge(row.reasoningEffortValue)}
                    {renderImageIntentBadge(row.imageIntentDisplay, t)}
                    {renderInvocationTransportBadge(row.record)}
                    {renderFastIndicator(row.fastIndicatorState, t)}
                  </div>
                )}
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

              <dl className="mt-3 space-y-2.5 text-xs text-base-content/75">
                {renderMobileFocus(row, isExpanded)}
              </dl>

              {isExpanded ? (
                <div className="mt-4 border-t border-base-300/60 pt-4" id={detailId}>
                  <div
                    className="min-w-0 max-w-full overflow-hidden space-y-4"
                    data-testid="records-expanded-detail-panel"
                  >
                    {renderDetailSummaryStrip(
                      row,
                      focus,
                      t,
                      costFormatter,
                      renderAccountValue,
                      "flat",
                    )}
                    {row.record.id > 0 ? (
                      <div className="flex justify-end">
                        <button
                          type="button"
                          className="inline-flex items-center gap-2 rounded-full border border-base-300/70 bg-base-100/70 px-3 py-1.5 text-xs font-medium text-base-content/78 transition hover:border-base-300 hover:bg-base-100"
                          onClick={() => setDrawerRecordId(row.record.id)}
                        >
                          <AppIcon
                            name="chevron-right-circle"
                            className="h-3.5 w-3.5"
                            aria-hidden
                          />
                          {t("table.responseBody.openFullDetails")}
                        </button>
                      </div>
                    ) : null}
                    <InvocationWorkflowDetailPanel
                      record={row.record}
                      focusedAttemptId={isExpanded ? focusedAttemptId : null}
                      size="compact"
                    />
                  </div>
                </div>
              ) : null}
            </article>
          );
        })}
      </div>

      <div className="hidden w-full min-w-0 max-w-full overflow-x-auto rounded-xl border border-base-300/70 bg-base-100/50 md:block">
        <table className="w-full table-fixed border-separate border-spacing-0 text-sm">
          <thead className="bg-base-200/65 text-[11px] uppercase tracking-[0.08em] text-base-content/70">
            <tr>
              <th className="px-3 py-3 text-left font-semibold">{t("table.column.time")}</th>
              <th className="px-3 py-3 text-left font-semibold">{t("table.column.proxy")}</th>
              <th className="px-3 py-3 text-left font-semibold">{t("table.column.model")}</th>
              <th className="px-3 py-3 text-left font-semibold">{t("table.column.status")}</th>
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

              return (
                <Fragment key={row.rowKey}>
                  <tr className={index % 2 === 0 ? "bg-base-100/30" : "bg-base-200/18"}>
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
                      {row.modelHasMismatch ? (
                        renderInvocationModelRoutingSummary({
                          requestModelValue: row.requestModelValue,
                          responseModelValue: row.responseModelValue,
                          hasMismatch: true,
                          t,
                          indicatorTestId: "invocation-records-model-routing-indicator",
                          adornments: (
                            <>
                              {renderReasoningEffortBadge(row.reasoningEffortValue)}
                              {renderImageIntentBadge(row.imageIntentDisplay, t)}
                              {renderInvocationTransportBadge(row.record)}
                              {renderFastIndicator(row.fastIndicatorState, t)}
                            </>
                          ),
                        })
                      ) : (
                        <div className="flex items-center gap-1" title={row.modelValue}>
                          {renderInvocationModelBadge(row.modelValue, {
                            t,
                            hasMismatch: false,
                            testId: "invocation-records-model",
                          })}
                          {renderReasoningEffortBadge(row.reasoningEffortValue)}
                          {renderImageIntentBadge(row.imageIntentDisplay, t)}
                          {renderInvocationTransportBadge(row.record)}
                          {renderFastIndicator(row.fastIndicatorState, t)}
                        </div>
                      )}
                    </td>
                    <td className="px-3 py-3 align-middle text-left text-xs">
                      <Badge variant={row.statusMeta.variant}>{row.statusLabel}</Badge>
                    </td>
                    {renderFocusCells(row, isExpanded)}
                    <td className="px-3 py-3 align-middle text-right">
                      <button
                        type="button"
                        className="inline-flex items-center justify-center rounded-md text-base-content/70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                        onClick={() =>
                          setExpandedId((current) => (current === row.rowKey ? null : row.rowKey))
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
                      <td colSpan={detailColSpan} className="max-w-0 px-4 py-4">
                        <div
                          className="min-w-0 max-w-full overflow-hidden space-y-3 rounded-xl border border-base-300/70 bg-base-200/45 p-3"
                          data-testid="records-expanded-detail-panel"
                        >
                          {renderDetailSummaryStrip(
                            row,
                            focus,
                            t,
                            costFormatter,
                            renderAccountValue,
                          )}
                          {row.record.id > 0 ? (
                            <div className="flex justify-end">
                              <button
                                type="button"
                                className="inline-flex items-center gap-2 rounded-full border border-base-300/70 bg-base-100/70 px-3 py-1.5 text-xs font-medium text-base-content/78 transition hover:border-base-300 hover:bg-base-100"
                                onClick={() => setDrawerRecordId(row.record.id)}
                              >
                                <AppIcon
                                  name="chevron-right-circle"
                                  className="h-3.5 w-3.5"
                                  aria-hidden
                                />
                                {t("table.responseBody.openFullDetails")}
                              </button>
                            </div>
                          ) : null}
                          <div id={detailId} className="min-w-0 max-w-full overflow-hidden">
                            <InvocationWorkflowDetailPanel
                              record={row.record}
                              focusedAttemptId={isExpanded ? focusedAttemptId : null}
                              size="default"
                            />
                          </div>
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
                <Badge variant={drawerRow.statusMeta.variant}>{drawerRow.statusLabel}</Badge>
                <span className="text-xs text-base-content/60">{drawerRow.occurredAtLabel}</span>
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
          {renderDetailSummaryStrip(drawerRow, focus, t, costFormatter, renderAccountValue)}

          <div className="rounded-xl border border-base-300/70 bg-base-200/35 p-4">
            <InvocationWorkflowDetailPanel
              record={drawerRow.record}
              focusedAttemptId={focusedAttemptId}
              size="default"
            />
          </div>
        </AccountDetailDrawerShell>
      ) : null}
    </div>
  );
}
