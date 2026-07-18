import { useEffect, useId, useMemo, useRef, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { Badge } from "../../components/ui/badge";
import { Spinner } from "../../components/ui/spinner";
import { useTranslation } from "../../i18n";
import type { ApiInvocation } from "../../lib/api";
import { fetchInvocationRecords } from "../../lib/api";
import {
  type DashboardWorkingConversationInvocationSelection,
  formatDashboardWorkingConversationSequenceId,
} from "../../lib/dashboardWorkingConversations";
import { resolveInvocationDisplayStatus } from "../../lib/invocationStatus";
import { AccountDetailDrawerShell } from "../account-pool/AccountDetailDrawerShell";
import { InvocationWorkflowDetailPanel } from "../invocations/InvocationWorkflowDetailPanel";
import { FALLBACK_CELL } from "../invocations/invocation-details-shared";
import { AppIcon } from "../shared/AppIcon";

interface DashboardInvocationDetailDrawerProps {
  open: boolean;
  invocationId?: string | null;
  selection: DashboardWorkingConversationInvocationSelection | null;
  onClose: () => void;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
}

const TRANSIENT_RECORD_LOOKUP_RETRY_MS = 1_500;

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
  if (lower === "warning_success") {
    return { variant: "warning", labelKey: "table.status.warningSuccess" };
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

function formatOccurredAtLabel(value: string, formatter: Intl.DateTimeFormat) {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value || FALLBACK_CELL;
  return formatter.format(parsed);
}

export function DashboardInvocationDetailDrawer({
  open,
  invocationId,
  selection,
  onClose,
  onOpenUpstreamAccount,
}: DashboardInvocationDetailDrawerProps) {
  const { t, locale } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const titleId = useId();
  const requestSeqRef = useRef(0);
  const [retryRevision, setRetryRevision] = useState(0);
  const [fullRecord, setFullRecord] = useState<ApiInvocation | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const effectiveInvocationId = invocationId ?? selection?.invocation.record.invokeId ?? null;
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
    if (!open || !effectiveInvocationId) {
      requestSeqRef.current += 1;
      setRetryRevision(0);
      setFullRecord(null);
      setIsLoading(false);
      setLoadError(null);
      return;
    }

    const selectedRecord =
      selection?.invocation.record.invokeId === effectiveInvocationId
        ? selection.invocation.record
        : null;
    const transientRecord =
      fullRecord?.invokeId === effectiveInvocationId ? fullRecord : selectedRecord;
    const isRetryLookup = retryRevision > 0 && transientRecord != null && !(transientRecord.id > 0);
    const requestSeq = requestSeqRef.current + 1;
    requestSeqRef.current = requestSeq;
    if (!isRetryLookup) {
      setFullRecord(null);
      setIsLoading(true);
    }
    setLoadError(null);

    void fetchInvocationRecords({
      requestId: effectiveInvocationId,
      pageSize: 1,
      sortBy: "occurredAt",
      sortOrder: "desc",
    })
      .then((response) => {
        if (requestSeq !== requestSeqRef.current) return;
        const exactRecord =
          response.records.find((record) => record.invokeId === effectiveInvocationId) ?? null;
        setFullRecord(exactRecord);
      })
      .catch((error) => {
        if (requestSeq !== requestSeqRef.current) return;
        setLoadError(error instanceof Error ? error.message : String(error));
      })
      .finally(() => {
        if (requestSeq === requestSeqRef.current && !isRetryLookup) {
          setIsLoading(false);
        }
      });
  }, [effectiveInvocationId, open, retryRevision]);

  const selectionRecord =
    selection?.invocation.record.invokeId === effectiveInvocationId
      ? selection.invocation.record
      : null;
  const effectiveTransientRecord =
    fullRecord?.invokeId === effectiveInvocationId ? fullRecord : selectionRecord;

  useEffect(() => {
    if (
      selectionRecord != null &&
      selectionRecord.id > 0 &&
      fullRecord != null &&
      !(fullRecord.id > 0) &&
      selectionRecord.invokeId === effectiveInvocationId
    ) {
      setFullRecord(selectionRecord);
      setIsLoading(false);
      setLoadError(null);
    }
  }, [effectiveInvocationId, fullRecord?.id, selectionRecord]);

  useEffect(() => {
    if (
      !open ||
      effectiveInvocationId == null ||
      effectiveTransientRecord == null ||
      effectiveTransientRecord.invokeId !== effectiveInvocationId ||
      effectiveTransientRecord.id > 0 ||
      isLoading ||
      loadError != null
    ) {
      return;
    }

    const retryTimer = window.setTimeout(() => {
      setRetryRevision((current) => current + 1);
    }, TRANSIENT_RECORD_LOOKUP_RETRY_MS);
    return () => window.clearTimeout(retryTimer);
  }, [effectiveInvocationId, effectiveTransientRecord, isLoading, loadError, open]);

  const recordForHeader = fullRecord ?? selectionRecord;
  const statusMeta = resolveStatusMeta(
    recordForHeader != null
      ? resolveInvocationDisplayStatus(recordForHeader)
      : selection?.invocation.displayStatus,
  );
  const statusLabel = statusMeta.labelKey
    ? t(statusMeta.labelKey as Parameters<typeof t>[0])
    : (statusMeta.label ?? t("table.status.unknown"));
  const slotLabel = selection
    ? selection.slotKind === "previous"
      ? t("dashboard.workingConversations.previousInvocation")
      : t("dashboard.workingConversations.currentInvocation")
    : t("dashboard.workingConversations.invocation");
  const occurredAtLabel =
    recordForHeader != null
      ? formatOccurredAtLabel(recordForHeader.occurredAt, dateTimeFormatter)
      : FALLBACK_CELL;
  const displaySequenceId =
    selection?.invocation.record.invokeId === effectiveInvocationId
      ? formatDashboardWorkingConversationSequenceId(selection.conversationSequenceId)
      : null;

  return (
    <AccountDetailDrawerShell
      open={open}
      labelledBy={titleId}
      closeLabel={t("dashboard.workingConversations.drawer.close")}
      onClose={onClose}
      shellClassName="max-w-[72rem]"
      header={
        <div className="space-y-3" data-testid="dashboard-invocation-detail-drawer">
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant="secondary">{slotLabel}</Badge>
            <Badge variant={statusMeta.variant}>{statusLabel}</Badge>
            {displaySequenceId ? <Badge variant="secondary">{displaySequenceId}</Badge> : null}
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
              {recordForHeader?.promptCacheKey ?? selection?.promptCacheKey ?? FALLBACK_CELL}
            </p>
            <p className="break-all font-mono text-xs text-base-content/58">
              {recordForHeader?.invokeId ?? effectiveInvocationId ?? FALLBACK_CELL}
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
          <Spinner size="sm" aria-label={t("dashboard.workingConversations.drawer.loading")} />
          <span className="text-sm text-base-content/70">
            {t("dashboard.workingConversations.drawer.loading")}
          </span>
        </div>
      ) : loadError ? (
        <Alert variant="error" data-testid="dashboard-invocation-detail-error">
          <AppIcon name="alert-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
          <div>
            <p className="font-medium">{t("dashboard.workingConversations.drawer.errorTitle")}</p>
            <p className="mt-1 text-sm">{loadError}</p>
          </div>
        </Alert>
      ) : !fullRecord ? (
        <div
          className="flex min-h-[18rem] flex-col items-center justify-center rounded-[1.6rem] border border-dashed border-base-300/80 bg-base-100/45 px-6 text-center"
          data-testid="dashboard-invocation-detail-empty"
        >
          <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-primary/10 text-primary">
            <AppIcon name="account-details-outline" className="h-7 w-7" aria-hidden />
          </div>
          <h3 className="text-lg font-semibold">
            {t("dashboard.workingConversations.drawer.emptyTitle")}
          </h3>
          <p className="mt-2 max-w-sm text-sm leading-6 text-base-content/65">
            {t("dashboard.workingConversations.drawer.emptyBody")}
          </p>
        </div>
      ) : (
        <div className="rounded-xl border border-base-300/70 bg-base-200/35 p-4">
          <InvocationWorkflowDetailPanel
            record={fullRecord}
            size="default"
            onOpenUpstreamAccount={onOpenUpstreamAccount}
          />
        </div>
      )}
    </AccountDetailDrawerShell>
  );
}
