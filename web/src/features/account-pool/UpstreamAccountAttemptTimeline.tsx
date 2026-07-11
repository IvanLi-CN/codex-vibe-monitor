import { Link } from "react-router-dom";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Alert } from "../../components/ui/alert";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { Spinner } from "../../components/ui/spinner";
import {
  fetchUpstreamAccountAttempts,
  locateUpstreamAccountAttempt,
  type ApiPoolUpstreamRequestAttempt,
  type UpstreamAccountAttemptListResponse,
} from "../../lib/api";
import { useTranslation } from "../../i18n";

const PAGE_SIZE = 50;

function attemptVariant(status: string) {
  if (status === "success") return "success" as const;
  if (status === "pending") return "warning" as const;
  return "error" as const;
}

function displayValue(value: string | number | null | undefined) {
  return value == null || value === "" ? "-" : String(value);
}

function formatLatency(value: number | null | undefined) {
  if (value == null || !Number.isFinite(value)) return "-";
  return value >= 1000
    ? `${(value / 1000).toFixed(2)} s`
    : `${Math.round(value)} ms`;
}

function AttemptField({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: ReactNode;
  mono?: boolean;
}) {
  return (
    <div className="min-w-0">
      <dt className="text-[11px] font-medium text-base-content/55">{label}</dt>
      <dd
        className={`mt-0.5 min-w-0 break-words text-sm text-base-content/85 ${mono ? "font-mono text-xs" : ""}`}
      >
        {value}
      </dd>
    </div>
  );
}

export function UpstreamAccountAttemptTimeline({
  accountId,
  focusedAttemptId,
}: {
  accountId: number;
  focusedAttemptId: number | null;
}) {
  const { t, locale } = useTranslation();
  const [response, setResponse] =
    useState<UpstreamAccountAttemptListResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [expandedAttemptId, setExpandedAttemptId] = useState<number | null>(null);
  const requestSeqRef = useRef(0);
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const costFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag, { style: "currency", currency: "USD", maximumFractionDigits: 4 }),
    [localeTag],
  );
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag),
    [localeTag],
  );
  const timeFormatter = useMemo(
    () => new Intl.DateTimeFormat(localeTag, { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false }),
    [localeTag],
  );

  useEffect(() => {
    const controller = new AbortController();
    const requestSeq = requestSeqRef.current + 1;
    requestSeqRef.current = requestSeq;
    setLoading(true);
    setError(null);
    const request = focusedAttemptId == null
      ? fetchUpstreamAccountAttempts(accountId, { page: 1, pageSize: PAGE_SIZE, signal: controller.signal })
      : locateUpstreamAccountAttempt(accountId, focusedAttemptId, { pageSize: PAGE_SIZE, signal: controller.signal });
    void request
      .then((next) => {
        if (controller.signal.aborted || requestSeq !== requestSeqRef.current) return;
        setResponse(next);
        setExpandedAttemptId(focusedAttemptId);
      })
      .catch((requestError) => {
        if (controller.signal.aborted || requestSeq !== requestSeqRef.current) return;
        setResponse(null);
        setError(
          focusedAttemptId != null && requestError instanceof Error && requestError.message.includes("404")
            ? t("accountPool.upstreamAttempts.locateUnavailable")
            : requestError instanceof Error ? requestError.message : String(requestError),
        );
      })
      .finally(() => {
        if (!controller.signal.aborted && requestSeq === requestSeqRef.current) setLoading(false);
      });
    return () => controller.abort();
  }, [accountId, focusedAttemptId, t]);

  const loadPage = (page: number) => {
    const requestSeq = requestSeqRef.current + 1;
    requestSeqRef.current = requestSeq;
    setLoading(true);
    setError(null);
    setExpandedAttemptId(null);
    void fetchUpstreamAccountAttempts(accountId, { page, pageSize: PAGE_SIZE })
      .then((next) => {
        if (requestSeq === requestSeqRef.current) setResponse(next);
      })
      .catch((requestError) => {
        if (requestSeq !== requestSeqRef.current) return;
        setError(requestError instanceof Error ? requestError.message : String(requestError));
      })
      .finally(() => {
        if (requestSeq === requestSeqRef.current) setLoading(false);
      });
  };

  if (loading && !response) return <div className="flex justify-center py-10"><Spinner /></div>;
  if (error) return <Alert variant="warning">{error}</Alert>;
  if (!response || response.items.length === 0) {
    return <p className="text-sm text-base-content/68">{t("accountPool.upstreamAttempts.empty")}</p>;
  }

  return (
    <section className="space-y-3" data-testid="upstream-account-call-records">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <p className="text-sm text-base-content/68">{t("accountPool.upstreamAttempts.description")}</p>
        <span className="shrink-0 text-sm tabular-nums text-base-content/58">
          {t("accountPool.upstreamAttempts.total", { count: response.total })}
        </span>
      </div>
      <div className="divide-y divide-base-300/70 overflow-hidden rounded-lg border border-base-300/70 bg-base-100/65">
        {response.items.map((attempt: ApiPoolUpstreamRequestAttempt) => {
          const expanded = attempt.id === expandedAttemptId;
          const attemptTime = new Date(attempt.occurredAt);
          const timeLabel = Number.isNaN(attemptTime.valueOf()) ? attempt.occurredAt : timeFormatter.format(attemptTime);
          const modelLabel = attempt.model?.trim() || t("accountPool.upstreamAttempts.modelUnavailable");
          const latencyLabel = [
            t("accountPool.upstreamAttempts.connect", { value: formatLatency(attempt.connectLatencyMs) }),
            t("accountPool.upstreamAttempts.firstByte", { value: formatLatency(attempt.firstByteLatencyMs) }),
            t("accountPool.upstreamAttempts.stream", { value: formatLatency(attempt.streamLatencyMs) }),
          ].join(" · ");
          return (
            <article
              key={attempt.id}
              data-attempt-id={attempt.id}
              data-testid={`account-attempt-record-${attempt.id}`}
              className={expanded ? "bg-info/5" : "bg-base-100/35"}
            >
              <div className="p-3 sm:p-4">
                <div className="flex flex-wrap items-center gap-2">
                  <Badge variant={attemptVariant(attempt.status)}>{attempt.status}</Badge>
                  {attempt.httpStatus != null ? <Badge variant="secondary">HTTP {attempt.httpStatus}</Badge> : null}
                  <span className="font-mono text-xs text-base-content/65">{t("accountPool.upstreamAttempts.index", { count: attempt.attemptIndex })}</span>
                  <span className="ml-auto text-xs tabular-nums text-base-content/58">{timeLabel}</span>
                </div>
                <div className="mt-2 flex flex-wrap items-baseline gap-x-3 gap-y-1">
                  <span className="break-all font-mono text-sm font-medium text-base-content">{attempt.invokeId}</span>
                  <span className="text-sm text-base-content/72">{modelLabel}</span>
                  <span className="font-mono text-xs text-base-content/62">{attempt.endpoint}</span>
                </div>
                <dl className="mt-3 grid gap-x-5 gap-y-3 sm:grid-cols-2 xl:grid-cols-4">
                  <AttemptField label={t("accountPool.upstreamAttempts.fields.retry")} value={t("accountPool.upstreamAttempts.retryValue", { same: attempt.sameAccountRetryIndex + 1, distinct: attempt.distinctAccountIndex + 1 })} />
                  <AttemptField label={t("accountPool.upstreamAttempts.fields.proxy")} value={displayValue(attempt.proxyBindingKeySnapshot)} mono />
                  <AttemptField label={t("accountPool.upstreamAttempts.fields.latency")} value={latencyLabel} mono />
                  <AttemptField label={t("accountPool.upstreamAttempts.fields.callSummary")} value={t("accountPool.upstreamAttempts.summaryValue", { tokens: attempt.totalTokens == null ? "-" : numberFormatter.format(attempt.totalTokens), cost: attempt.cost == null ? "-" : costFormatter.format(attempt.cost) })} mono />
                </dl>
                <div className="mt-3 flex flex-wrap items-center gap-x-4 gap-y-2">
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    aria-expanded={expanded}
                    onClick={() => setExpandedAttemptId((current) => current === attempt.id ? null : attempt.id)}
                  >
                    {expanded ? t("accountPool.upstreamAttempts.collapse") : t("accountPool.upstreamAttempts.expand")}
                  </Button>
                  <Link className="text-sm text-info underline underline-offset-4 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary" to={`/records?requestId=${encodeURIComponent(attempt.invokeId)}&rangePreset=7d`}>
                    {t("accountPool.upstreamAttempts.globalOverview")}
                  </Link>
                </div>
                {expanded ? (
                  <dl className="mt-3 grid gap-x-5 gap-y-3 border-t border-base-300/70 pt-3 sm:grid-cols-2 xl:grid-cols-4">
                    <AttemptField label={t("accountPool.upstreamAttempts.fields.failureKind")} value={displayValue(attempt.failureKind)} mono />
                    <AttemptField label={t("accountPool.upstreamAttempts.fields.downstreamHttp")} value={attempt.downstreamHttpStatus == null ? "-" : `HTTP ${attempt.downstreamHttpStatus}`} mono />
                    <AttemptField label={t("accountPool.upstreamAttempts.fields.requesterIp")} value={displayValue(attempt.requesterIp)} mono />
                    <AttemptField label={t("accountPool.upstreamAttempts.fields.stickyKey")} value={displayValue(attempt.stickyKey)} mono />
                    <AttemptField label={t("accountPool.upstreamAttempts.fields.route")} value={displayValue(attempt.upstreamRouteKey)} mono />
                    <AttemptField label={t("accountPool.upstreamAttempts.fields.upstreamRequestId")} value={displayValue(attempt.upstreamRequestId)} mono />
                    {attempt.errorMessage ? <AttemptField label={t("accountPool.upstreamAttempts.fields.error")} value={attempt.errorMessage} /> : null}
                  </dl>
                ) : null}
              </div>
            </article>
          );
        })}
      </div>
      <div className="flex items-center justify-end gap-2">
        <Button variant="outline" size="sm" disabled={loading || response.page <= 1} onClick={() => loadPage(response.page - 1)}>{t("accountPool.upstreamAttempts.previous")}</Button>
        <Button variant="outline" size="sm" disabled={loading || response.page * response.pageSize >= response.total} onClick={() => loadPage(response.page + 1)}>{t("accountPool.upstreamAttempts.next")}</Button>
      </div>
    </section>
  );
}
