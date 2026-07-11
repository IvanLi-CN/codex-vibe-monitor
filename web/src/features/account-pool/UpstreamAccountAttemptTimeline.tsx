import { Link } from "react-router-dom";
import { useEffect, useRef, useState } from "react";
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

function formatAttemptTime(value: string) {
  const parsed = new Date(value);
  return Number.isNaN(parsed.valueOf()) ? value : parsed.toLocaleString();
}

export function UpstreamAccountAttemptTimeline({
  accountId,
  focusedAttemptId,
}: {
  accountId: number;
  focusedAttemptId: number | null;
}) {
  const { t } = useTranslation();
  const [response, setResponse] =
    useState<UpstreamAccountAttemptListResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const requestSeqRef = useRef(0);

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
          focusedAttemptId != null &&
            requestError instanceof Error &&
            requestError.message.includes("404")
            ? t("accountPool.upstreamAttempts.locateUnavailable")
            : requestError instanceof Error
              ? requestError.message
              : String(requestError),
        );
      })
      .finally(() => {
        if (!controller.signal.aborted && requestSeq === requestSeqRef.current) {
          setLoading(false);
        }
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

  if (loading && !response) {
    return <div className="flex justify-center py-10"><Spinner /></div>;
  }
  if (error) return <Alert variant="warning">{error}</Alert>;
  if (!response || response.items.length === 0) {
    return <p className="text-sm text-base-content/68">{t("accountPool.upstreamAttempts.empty")}</p>;
  }

  return (
    <div className="space-y-3" data-testid="upstream-account-attempt-timeline">
      <p className="text-sm text-base-content/68">{t("accountPool.upstreamAttempts.description")}</p>
      {response.items.map((attempt: ApiPoolUpstreamRequestAttempt) => {
        const focused = attempt.id === focusedAttemptId;
        return (
          <article
            key={attempt.id}
            data-attempt-id={attempt.id}
            className={`border p-3 ${focused ? "border-info bg-info/10" : "border-base-300/70 bg-base-100/70"}`}
          >
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant={attemptVariant(attempt.status)}>{attempt.status}</Badge>
              {attempt.httpStatus != null ? <Badge variant="secondary">HTTP {attempt.httpStatus}</Badge> : null}
              <span className="text-xs text-base-content/58">{formatAttemptTime(attempt.occurredAt)}</span>
              <span className="text-xs text-base-content/58">
                {t("accountPool.upstreamAttempts.index", { count: attempt.attemptIndex })}
              </span>
            </div>
            <p className="mt-2 break-all font-mono text-xs text-base-content/80">{attempt.invokeId}</p>
            {attempt.errorMessage ? <p className="mt-2 break-words text-sm text-base-content/72">{attempt.errorMessage}</p> : null}
            <Link
              className="mt-3 inline-block text-sm text-info underline underline-offset-4"
              to={`/records?requestId=${encodeURIComponent(attempt.invokeId)}&rangePreset=7d`}
            >
              {t("accountPool.upstreamAttempts.globalOverview")}
            </Link>
          </article>
        );
      })}
      <div className="flex items-center justify-between gap-3 text-sm text-base-content/68">
        <span>{t("accountPool.upstreamAttempts.total", { count: response.total })}</span>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" disabled={loading || response.page <= 1} onClick={() => loadPage(response.page - 1)}>
            {t("accountPool.upstreamAttempts.previous")}
          </Button>
          <Button variant="outline" size="sm" disabled={loading || response.page * response.pageSize >= response.total} onClick={() => loadPage(response.page + 1)}>
            {t("accountPool.upstreamAttempts.next")}
          </Button>
        </div>
      </div>
    </div>
  );
}
