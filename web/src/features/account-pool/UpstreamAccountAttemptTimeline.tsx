import { Link } from "react-router-dom";
import { useEffect, useMemo, useRef, useState } from "react";
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

function statusLabel(status: string, t: (key: string) => string) {
  const known = new Set(["success", "pending", "http_failure", "transport_failure", "failed"]);
  return known.has(status) ? t(`accountPool.upstreamAttempts.status.${status}`) : status;
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
  const requestSeqRef = useRef(0);
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const timeFormatter = useMemo(
    () => new Intl.DateTimeFormat(localeTag, {
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
    const controller = new AbortController();
    const requestSeq = requestSeqRef.current + 1;
    requestSeqRef.current = requestSeq;
    setLoading(true);
    setError(null);
    const request = focusedAttemptId == null
      ? fetchUpstreamAccountAttempts(accountId, {
        page: 1,
        pageSize: PAGE_SIZE,
        signal: controller.signal,
      })
      : locateUpstreamAccountAttempt(accountId, focusedAttemptId, {
        pageSize: PAGE_SIZE,
        signal: controller.signal,
      });
    void request
      .then((next) => {
        if (controller.signal.aborted || requestSeq !== requestSeqRef.current) return;
        setResponse(next);
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
      <div className="overflow-x-auto rounded-lg border border-base-300/70 bg-base-100/65">
        <table className="min-w-full w-max border-collapse text-left text-sm" data-testid="upstream-account-call-records-table">
          <thead className="border-b border-base-300/70 bg-base-200/55 text-xs font-medium text-base-content/65">
            <tr>
              <th className="w-36 px-3 py-2.5">{t("accountPool.upstreamAttempts.columns.time")}</th>
              <th className="whitespace-nowrap px-3 py-2.5">{t("accountPool.upstreamAttempts.columns.call")}</th>
              <th className="w-32 px-3 py-2.5">{t("accountPool.upstreamAttempts.columns.model")}</th>
              <th className="w-36 px-3 py-2.5">{t("accountPool.upstreamAttempts.columns.endpoint")}</th>
              <th className="w-36 px-3 py-2.5">{t("accountPool.upstreamAttempts.columns.result")}</th>
              <th className="w-36 px-3 py-2.5">{t("accountPool.upstreamAttempts.columns.proxy")}</th>
              <th className="w-48 px-3 py-2.5">{t("accountPool.upstreamAttempts.columns.timing")}</th>
              <th className="min-w-56 px-3 py-2.5">{t("accountPool.upstreamAttempts.columns.error")}</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-base-300/70">
            {response.items.map((attempt: ApiPoolUpstreamRequestAttempt) => {
              const attemptTime = new Date(attempt.occurredAt);
              const timeLabel = Number.isNaN(attemptTime.valueOf())
                ? attempt.occurredAt
                : timeFormatter.format(attemptTime);
              const rowFocused = attempt.id === focusedAttemptId;
              return (
                <tr
                  key={attempt.id}
                  data-attempt-id={attempt.id}
                  data-testid={`account-attempt-record-${attempt.id}`}
                  className={rowFocused ? "bg-info/10" : "hover:bg-base-200/35"}
                >
                  <td className="whitespace-nowrap px-3 py-3 align-top font-mono text-xs tabular-nums text-base-content/70">
                    {timeLabel}
                  </td>
                  <td className="whitespace-nowrap px-3 py-3 align-top font-mono text-xs">
                    <Link
                      className="inline-block whitespace-nowrap text-info underline underline-offset-4 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                      to={`/records?requestId=${encodeURIComponent(attempt.invokeId)}&rangePreset=7d`}
                      title={attempt.invokeId}
                    >
                      {attempt.invokeId}
                    </Link>
                  </td>
                  <td className="break-words px-3 py-3 align-top text-base-content/80">
                    {attempt.model?.trim() || t("accountPool.upstreamAttempts.modelUnavailable")}
                  </td>
                  <td className="break-all px-3 py-3 align-top font-mono text-xs text-base-content/75">
                    {attempt.endpoint}
                  </td>
                  <td className="px-3 py-3 align-top">
                    <div className="flex flex-wrap items-center gap-1.5">
                      <Badge variant={attemptVariant(attempt.status)}>{statusLabel(attempt.status, t)}</Badge>
                      {attempt.httpStatus != null ? <span className="font-mono text-xs text-base-content/70">HTTP {attempt.httpStatus}</span> : null}
                    </div>
                  </td>
                  <td className="break-all px-3 py-3 align-top font-mono text-xs text-base-content/75">
                    {displayValue(attempt.proxyBindingKeySnapshot)}
                  </td>
                  <td className="px-3 py-3 align-top font-mono text-xs tabular-nums text-base-content/75">
                    <div>{t("accountPool.upstreamAttempts.connect", { value: formatLatency(attempt.connectLatencyMs) })}</div>
                    <div>{t("accountPool.upstreamAttempts.firstByte", { value: formatLatency(attempt.firstByteLatencyMs) })}</div>
                    <div>{t("accountPool.upstreamAttempts.stream", { value: formatLatency(attempt.streamLatencyMs) })}</div>
                  </td>
                  <td className="break-words px-3 py-3 align-top text-xs leading-5 text-base-content/75">
                    {attempt.failureKind ? <span className="font-mono text-base-content/85">{attempt.failureKind}</span> : null}
                    {attempt.failureKind && attempt.errorMessage ? <span className="text-base-content/45"> · </span> : null}
                    {attempt.errorMessage || (attempt.failureKind ? null : "-")}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <div className="flex items-center justify-end gap-2">
        <Button variant="outline" size="sm" disabled={loading || response.page <= 1} onClick={() => loadPage(response.page - 1)}>{t("accountPool.upstreamAttempts.previous")}</Button>
        <Button variant="outline" size="sm" disabled={loading || response.page * response.pageSize >= response.total} onClick={() => loadPage(response.page + 1)}>{t("accountPool.upstreamAttempts.next")}</Button>
      </div>
    </section>
  );
}
