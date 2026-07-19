import { useEffect, useMemo, useRef, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { Button } from "../../components/ui/button";
import { Spinner } from "../../components/ui/spinner";
import { useForwardProxyBindingNodes } from "../../hooks/useForwardProxyBindingNodes";
import { useTranslation } from "../../i18n";
import type {
  ApiInvocation,
  ApiInvocationWorkflowTimelineEntry,
  ApiPoolUpstreamRequestAttempt,
  ForwardProxyBindingNode,
  UpstreamAccountAttemptListResponse,
} from "../../lib/api";
import { fetchUpstreamAccountAttempts, locateUpstreamAccountAttempt } from "../../lib/api";
import { InvocationWorkflowAttemptRecord } from "../invocations/InvocationWorkflowDetailPanel";

const PAGE_SIZE = 50;
const CALL_SHORT_ID_PATTERN = /^[ABCDEFGHJKMNPQRSTUVWXYZ23456789]{10}$/;

function compactProxyBindingKey(value: string) {
  if (value.length <= 18) return value;
  return `${value.slice(0, 8)}...${value.slice(-6)}`;
}

function displayCallShortId(invokeId: string | null | undefined) {
  const normalized = invokeId?.trim().toUpperCase() ?? "";
  if (!normalized) return null;
  if (CALL_SHORT_ID_PATTERN.test(normalized)) return normalized;
  return null;
}

function collectProxyBindingKeys(items: ApiPoolUpstreamRequestAttempt[] | undefined) {
  return Array.from(
    new Set(
      (items ?? [])
        .map((item) => item.proxyBindingKeySnapshot?.trim() ?? "")
        .filter((key) => key.length > 0 && key !== "__direct__"),
    ),
  ).sort((left, right) => left.localeCompare(right));
}

function buildProxyBindingNodeMap(nodes: ForwardProxyBindingNode[]) {
  const entries = new Map<string, ForwardProxyBindingNode>();
  for (const node of nodes) {
    entries.set(node.key, node);
    for (const aliasKey of node.aliasKeys ?? []) entries.set(aliasKey, node);
  }
  return entries;
}

function formatProxyBinding(
  attempt: ApiPoolUpstreamRequestAttempt,
  nodesByKey: Map<string, ForwardProxyBindingNode>,
  proxyDirectLabel: string,
) {
  const key = attempt.proxyBindingKeySnapshot?.trim();
  if (!key) return { value: "-", title: "-", resolved: false };
  if (key === "__direct__") {
    return { value: proxyDirectLabel, title: proxyDirectLabel, resolved: true };
  }
  const displayName = nodesByKey.get(key)?.displayName.trim();
  if (displayName && displayName !== key) {
    return {
      value: displayName,
      title: `${displayName} (${key})`,
      resolved: true,
    };
  }
  return { value: compactProxyBindingKey(key), title: key, resolved: false };
}

function buildSyntheticInvocationRecord(attempt: ApiPoolUpstreamRequestAttempt): ApiInvocation {
  return {
    id: 0,
    invokeId: attempt.invokeId,
    occurredAt: attempt.occurredAt,
    endpoint: attempt.endpoint,
    requestModel: attempt.requestModel ?? attempt.model ?? undefined,
    responseModel: attempt.responseModel ?? undefined,
    status: attempt.status,
    requesterIp: attempt.requesterIp ?? undefined,
    upstreamAccountId: attempt.upstreamAccountId ?? undefined,
    upstreamAccountName: attempt.upstreamAccountName ?? undefined,
    upstreamRequestId: attempt.upstreamRequestId ?? undefined,
    routeMode: "pool",
    createdAt: attempt.createdAt,
  };
}

function buildSyntheticWorkflowAttemptEntry(
  attempt: ApiPoolUpstreamRequestAttempt,
  proxyDisplay: ReturnType<typeof formatProxyBinding>,
): ApiInvocationWorkflowTimelineEntry {
  const requestSummary: Record<string, unknown> = {
    requestModel: attempt.requestModel ?? attempt.model ?? null,
    responseModel: attempt.responseModel ?? null,
    endpoint: attempt.endpoint ?? null,
    routing: {
      routeMode: "pool",
      proxyDisplayName: proxyDisplay.value !== "-" ? proxyDisplay.value : null,
      upstreamRouteKey: attempt.upstreamRouteKey ?? null,
      proxyBindingKey: attempt.proxyBindingKeySnapshot ?? null,
      stickyKey: attempt.stickyKey ?? null,
    },
    compression: {
      algorithm: attempt.upstreamRequestCompressionAlgorithm ?? null,
      mode: attempt.upstreamRequestCompressionMode ?? null,
      logicalBodyBytes: attempt.logicalBodyBytes ?? null,
      transmittedBodyBytes: attempt.transmittedBodyBytes ?? null,
      ratioPct: attempt.ratioPct ?? null,
      approxUploadBytes: attempt.approxUploadBytes ?? null,
      approxDownloadBytes: attempt.approxDownloadBytes ?? null,
    },
  };
  const requestBodySize =
    attempt.logicalBodyBytes ?? attempt.transmittedBodyBytes ?? attempt.approxUploadBytes ?? null;
  if (requestBodySize != null) {
    requestSummary.bodyCapture = { size: requestBodySize };
  }

  const responseSummary: Record<string, unknown> = {
    status: attempt.status,
    failureKind: attempt.failureKind ?? null,
    errorMessage: attempt.errorMessage ?? null,
    downstreamErrorMessage: attempt.downstreamErrorMessage ?? null,
  };
  if (attempt.approxDownloadBytes != null) {
    responseSummary.responseBodyCapture = { size: attempt.approxDownloadBytes };
  }

  return {
    blockId: `upstream-account-attempt-${attempt.attemptId}`,
    kind: "attempt",
    occurredAt: attempt.occurredAt,
    title: "",
    status: attempt.status,
    attempt: {
      synthetic: false,
      attemptId: attempt.attemptId,
      occurredAt: attempt.occurredAt,
      endpoint: attempt.endpoint,
      stickyKey: attempt.stickyKey ?? null,
      upstreamAccountId: attempt.upstreamAccountId ?? null,
      upstreamAccountName: attempt.upstreamAccountName ?? null,
      requestModel: attempt.requestModel ?? attempt.model ?? null,
      responseModel: attempt.responseModel ?? null,
      upstreamRouteKey: attempt.upstreamRouteKey ?? null,
      proxyBindingKeySnapshot: attempt.proxyBindingKeySnapshot ?? null,
      attemptIndex: attempt.attemptIndex,
      distinctAccountIndex: attempt.distinctAccountIndex,
      sameAccountRetryIndex: attempt.sameAccountRetryIndex,
      requesterIp: attempt.requesterIp ?? (proxyDisplay.value !== "-" ? proxyDisplay.value : null),
      startedAt: attempt.startedAt ?? null,
      finishedAt: attempt.finishedAt ?? null,
      status: attempt.status,
      phase: attempt.phase ?? null,
      httpStatus: attempt.httpStatus ?? null,
      downstreamHttpStatus: attempt.downstreamHttpStatus ?? null,
      failureKind: attempt.failureKind ?? null,
      errorMessage: attempt.errorMessage ?? null,
      downstreamErrorMessage: attempt.downstreamErrorMessage ?? null,
      connectLatencyMs: attempt.connectLatencyMs ?? null,
      firstByteLatencyMs: attempt.firstByteLatencyMs ?? null,
      streamLatencyMs: attempt.streamLatencyMs ?? null,
      upstreamRequestId: attempt.upstreamRequestId ?? null,
      requestSummary,
      responseSummary,
    },
    detail: null,
    responseBody: null,
  };
}

export function UpstreamAccountAttemptTimeline({
  accountId,
  focusedAttemptId,
}: {
  accountId: number;
  focusedAttemptId: string | null;
}) {
  const { t, locale } = useTranslation();
  const [response, setResponse] = useState<UpstreamAccountAttemptListResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const requestSeqRef = useRef(0);
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const isZh = locale === "zh";
  const proxyDirectLabel = t("accountPool.upstreamAttempts.proxyDirect");
  const proxyBindingKeys = useMemo(
    () => collectProxyBindingKeys(response?.items),
    [response?.items],
  );
  const { nodes: proxyBindingNodes } = useForwardProxyBindingNodes(proxyBindingKeys, {
    enabled: proxyBindingKeys.length > 0,
  });
  const proxyBindingNodesByKey = useMemo(
    () => buildProxyBindingNodeMap(proxyBindingNodes),
    [proxyBindingNodes],
  );

  useEffect(() => {
    const controller = new AbortController();
    const requestSeq = requestSeqRef.current + 1;
    requestSeqRef.current = requestSeq;
    setLoading(true);
    setError(null);
    const request =
      focusedAttemptId == null
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
        if (requestSeq === requestSeqRef.current) {
          setError(requestError instanceof Error ? requestError.message : String(requestError));
        }
      })
      .finally(() => {
        if (requestSeq === requestSeqRef.current) setLoading(false);
      });
  };

  if (loading && !response) {
    return (
      <div className="flex justify-center py-10">
        <Spinner />
      </div>
    );
  }
  if (error) return <Alert variant="warning">{error}</Alert>;
  if (!response || response.items.length === 0) {
    return (
      <p className="text-sm text-base-content/68">{t("accountPool.upstreamAttempts.empty")}</p>
    );
  }

  return (
    <section className="space-y-3" data-testid="upstream-account-call-records">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <p className="text-sm text-base-content/68">
          {t("accountPool.upstreamAttempts.description")}
        </p>
        <span className="shrink-0 text-sm tabular-nums text-base-content/58">
          {t("accountPool.upstreamAttempts.total", { count: response.total })}
        </span>
      </div>
      <div className="space-y-3" data-testid="upstream-account-attempt-list">
        {response.items.map((attempt) => {
          const proxyDisplay = formatProxyBinding(
            attempt,
            proxyBindingNodesByKey,
            proxyDirectLabel,
          );
          const callShortId = displayCallShortId(attempt.invokeId);
          const syntheticEntry = buildSyntheticWorkflowAttemptEntry(attempt, proxyDisplay);
          const syntheticRecord = buildSyntheticInvocationRecord(attempt);
          const isFocused = attempt.attemptId === focusedAttemptId;

          return (
            <InvocationWorkflowAttemptRecord
              key={attempt.attemptId}
              record={syntheticRecord}
              entry={syntheticEntry}
              localeTag={localeTag}
              isZh={isZh}
              summaryIdentity={callShortId ?? attempt.attemptId}
              focused={isFocused}
              defaultSection={isFocused ? "timing" : null}
              testId={`account-attempt-record-${attempt.attemptId}`}
            />
          );
        })}
      </div>
      <div className="flex items-center justify-end gap-2">
        <Button
          variant="outline"
          size="sm"
          disabled={loading || response.page <= 1}
          onClick={() => loadPage(response.page - 1)}
        >
          {t("accountPool.upstreamAttempts.previous")}
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={loading || response.page * response.pageSize >= response.total}
          onClick={() => loadPage(response.page + 1)}
        >
          {t("accountPool.upstreamAttempts.next")}
        </Button>
      </div>
    </section>
  );
}
