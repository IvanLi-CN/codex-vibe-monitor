import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { Alert } from "../../components/ui/alert";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import { Spinner } from "../../components/ui/spinner";
import { Tooltip } from "../../components/ui/tooltip";
import { useForwardProxyBindingNodes } from "../../hooks/useForwardProxyBindingNodes";
import { useTranslation } from "../../i18n";
import {
  type ApiPoolUpstreamRequestAttempt,
  type ForwardProxyBindingNode,
  fetchUpstreamAccountAttempts,
  locateUpstreamAccountAttempt,
  type UpstreamAccountAttemptListResponse,
} from "../../lib/api";
import {
  requestCompressionAlgorithmLabel,
  requestCompressionModeLabel,
} from "../../lib/requestCompression";
import { AppIcon } from "../shared/AppIcon";

const PAGE_SIZE = 50;

type Translator = ReturnType<typeof useTranslation>["t"];

function attemptVariant(status: string) {
  if (status === "success") return "success" as const;
  if (status === "pending") return "warning" as const;
  return "error" as const;
}

function formatLatency(value: number | null | undefined) {
  if (value == null || !Number.isFinite(value)) return "-";
  return value >= 1000 ? `${(value / 1000).toFixed(2)} s` : `${Math.round(value)} ms`;
}

function statusLabel(status: string, t: Translator) {
  const known = new Set(["success", "pending", "http_failure", "transport_failure", "failed"]);
  return known.has(status) ? t(`accountPool.upstreamAttempts.status.${status}`) : status;
}

function pendingPhaseLabel(phase: string | null | undefined, t: Translator) {
  switch (phase?.trim().toLowerCase()) {
    case "connecting":
      return t("accountPool.upstreamAttempts.phase.connecting");
    case "waiting_first_byte":
      return t("accountPool.upstreamAttempts.phase.waitingFirstByte");
    case "streaming_response":
      return t("accountPool.upstreamAttempts.phase.streamingResponse");
    case "sending_request":
      return t("accountPool.upstreamAttempts.phase.sendingRequest");
    default:
      return t("accountPool.upstreamAttempts.phase.sendingRequest");
  }
}

function compressionAlgorithmValueLabel(value: string | null | undefined, t: Translator) {
  return requestCompressionAlgorithmLabel(
    value === "follow" ||
      value === "identity" ||
      value === "gzip" ||
      value === "deflate" ||
      value === "zstd"
      ? value
      : "identity",
    {
      requestCompressionFollow: t("accountPool.requestCompression.follow"),
      requestCompressionIdentity: t("accountPool.requestCompression.identity"),
      requestCompressionGzip: t("accountPool.requestCompression.gzip"),
      requestCompressionDeflate: t("accountPool.requestCompression.deflate"),
      requestCompressionZstd: t("accountPool.requestCompression.zstd"),
    },
  );
}

function compressionModeValueLabel(value: string | null | undefined, t: Translator) {
  return requestCompressionModeLabel(value, {
    requestCompressionModeIdentity: t("accountPool.requestCompression.mode.identity"),
    requestCompressionModePassthrough: t("accountPool.requestCompression.mode.passthrough"),
    requestCompressionModeRecompressed: t("accountPool.requestCompression.mode.recompressed"),
  });
}

function compactProxyBindingKey(value: string) {
  if (value.length <= 18) return value;
  return `${value.slice(0, 8)}...${value.slice(-6)}`;
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
  t: Translator,
) {
  const key = attempt.proxyBindingKeySnapshot?.trim();
  if (!key) return { value: "-", title: "-", resolved: false };
  if (key === "__direct__") {
    const value = t("accountPool.upstreamAttempts.proxyDirect");
    return { value, title: value, resolved: true };
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

function ModelMapping({ attempt, t }: { attempt: ApiPoolUpstreamRequestAttempt; t: Translator }) {
  const requestModel =
    attempt.requestModel?.trim() ||
    attempt.model?.trim() ||
    t("accountPool.upstreamAttempts.modelUnavailable");
  const responseModel = attempt.responseModel?.trim() || "-";
  return (
    <div className="text-xs text-base-content/80">
      <div className="whitespace-nowrap">
        <span className="text-base-content/55">
          {t("accountPool.upstreamAttempts.requestModel")}
        </span>
        <span className="ml-1 font-mono">{requestModel}</span>
      </div>
      <div className="mt-1 whitespace-nowrap">
        <span className="text-base-content/55">
          {t("accountPool.upstreamAttempts.responseModel")}
        </span>
        <span className="ml-1 font-mono">{responseModel}</span>
      </div>
    </div>
  );
}

function AttemptResult({ attempt, t }: { attempt: ApiPoolUpstreamRequestAttempt; t: Translator }) {
  const isPending = attempt.status === "pending";
  return (
    <div className="space-y-1">
      <div className="flex flex-wrap items-center gap-1.5">
        <Badge variant={attemptVariant(attempt.status)}>{statusLabel(attempt.status, t)}</Badge>
        {attempt.httpStatus != null ? (
          <span className="font-mono text-xs text-base-content/70">
            {t("accountPool.upstreamAttempts.upstreamHttp", {
              status: attempt.httpStatus,
            })}
          </span>
        ) : null}
      </div>
      {isPending ? (
        <p className="text-xs text-base-content/60">{pendingPhaseLabel(attempt.phase, t)}</p>
      ) : null}
    </div>
  );
}

function AttemptTiming({
  attempt,
  compact = false,
  t,
}: {
  attempt: ApiPoolUpstreamRequestAttempt;
  compact?: boolean;
  t: Translator;
}) {
  if (compact) {
    return (
      <span>
        {t("accountPool.upstreamAttempts.timingCompact", {
          connect: formatLatency(attempt.connectLatencyMs),
          firstByte: formatLatency(attempt.firstByteLatencyMs),
          stream: formatLatency(attempt.streamLatencyMs),
        })}
      </span>
    );
  }
  return (
    <>
      <div>
        {t("accountPool.upstreamAttempts.connect", {
          value: formatLatency(attempt.connectLatencyMs),
        })}
      </div>
      <div>
        {t("accountPool.upstreamAttempts.firstByte", {
          value: formatLatency(attempt.firstByteLatencyMs),
        })}
      </div>
      <div>
        {t("accountPool.upstreamAttempts.stream", {
          value: formatLatency(attempt.streamLatencyMs),
        })}
      </div>
    </>
  );
}

function AttemptEvidenceDisclosure({
  attempt,
  proxy,
  includeTimings,
  isFocused,
  t,
}: {
  attempt: ApiPoolUpstreamRequestAttempt;
  proxy: ReturnType<typeof formatProxyBinding>;
  includeTimings: boolean;
  isFocused: boolean;
  t: Translator;
}) {
  const [copied, setCopied] = useState(false);
  const [isOpen, setIsOpen] = useState(isFocused);
  const errorMessage = attempt.errorMessage?.trim() || "";
  const downstreamDiffers =
    attempt.downstreamHttpStatus != null && attempt.downstreamHttpStatus !== attempt.httpStatus;
  const metadataItemClass = "flex items-baseline gap-1.5";
  const copyError = async () => {
    if (!errorMessage || !navigator.clipboard?.writeText) return;
    try {
      await navigator.clipboard.writeText(errorMessage);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    } catch {
      setCopied(false);
    }
  };
  useEffect(() => {
    if (isFocused) setIsOpen(true);
  }, [isFocused]);
  return (
    <details
      className="group overflow-hidden rounded-md border border-base-300/70 text-xs"
      data-testid={`account-attempt-evidence-${attempt.attemptId}`}
      onToggle={(event) => setIsOpen(event.currentTarget.open)}
      open={isOpen}
    >
      <summary className="flex min-h-11 cursor-pointer list-none select-none items-center gap-2 px-3 py-2 font-medium text-info focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-primary [&::-webkit-details-marker]:hidden">
        <AppIcon
          aria-hidden
          className="h-4 w-4 shrink-0 transition-transform duration-150 group-open:rotate-90"
          name="chevron-right"
        />
        {t("accountPool.upstreamAttempts.details")}
      </summary>
      <div className="space-y-2 border-t border-base-300/70 bg-base-200/35 px-3 py-2.5 text-base-content/75">
        <dl className="flex flex-wrap items-baseline gap-x-5 gap-y-1">
          <div className={metadataItemClass}>
            <dt className="text-base-content/55">
              {t("accountPool.upstreamAttempts.proxyBinding")}
            </dt>
            <dd className={proxy.resolved ? "font-medium" : "font-mono"} title={proxy.title}>
              {proxy.value}
            </dd>
          </div>
          {attempt.upstreamRequestId?.trim() ? (
            <div className={metadataItemClass}>
              <dt className="text-base-content/55">
                {t("accountPool.upstreamAttempts.upstreamRequestId")}
              </dt>
              <dd className="break-all font-mono">{attempt.upstreamRequestId}</dd>
            </div>
          ) : null}
          {attempt.upstreamRouteKey?.trim() ? (
            <div className={metadataItemClass}>
              <dt className="text-base-content/55">{t("accountPool.upstreamAttempts.routeKey")}</dt>
              <dd className="break-all font-mono">{attempt.upstreamRouteKey}</dd>
            </div>
          ) : null}
          {downstreamDiffers ? (
            <div className={metadataItemClass}>
              <dt className="text-base-content/55">
                {t("accountPool.upstreamAttempts.downstreamHttp")}
              </dt>
              <dd className="font-mono">{attempt.downstreamHttpStatus}</dd>
            </div>
          ) : null}
          <div className={metadataItemClass}>
            <dt className="text-base-content/55">
              {t("accountPool.upstreamAttempts.downstreamRequestCompression")}
            </dt>
            <dd>{compressionAlgorithmValueLabel(attempt.downstreamRequestContentEncoding, t)}</dd>
          </div>
          <div className={metadataItemClass}>
            <dt className="text-base-content/55">
              {t("accountPool.upstreamAttempts.upstreamRequestCompression")}
            </dt>
            <dd>
              {compressionAlgorithmValueLabel(attempt.upstreamRequestCompressionAlgorithm, t)}
            </dd>
          </div>
          <div className={metadataItemClass}>
            <dt className="text-base-content/55">
              {t("accountPool.upstreamAttempts.upstreamRequestCompressionMode")}
            </dt>
            <dd>{compressionModeValueLabel(attempt.upstreamRequestCompressionMode, t)}</dd>
          </div>
          {includeTimings ? (
            <div className={metadataItemClass}>
              <dt className="text-base-content/55">
                {t("accountPool.upstreamAttempts.columns.timing")}
              </dt>
              <dd className="font-mono tabular-nums">
                <AttemptTiming attempt={attempt} compact t={t} />
              </dd>
            </div>
          ) : null}
        </dl>
        {errorMessage ? (
          <div
            className={
              includeTimings
                ? "border-t border-base-300/70 pt-2"
                : "flex min-w-0 items-start gap-2 border-t border-base-300/70 pt-2"
            }
          >
            <div className="flex shrink-0 items-center gap-1.5">
              <p className="text-base-content/55">{t("accountPool.upstreamAttempts.fullError")}</p>
              <Tooltip
                content={
                  copied
                    ? t("accountPool.upstreamAttempts.copied")
                    : t("accountPool.upstreamAttempts.copyError")
                }
              >
                <Button
                  aria-label={t("accountPool.upstreamAttempts.copyError")}
                  className="h-7 w-7"
                  size="icon"
                  type="button"
                  variant="ghost"
                  onClick={(event) => {
                    event.stopPropagation();
                    void copyError();
                  }}
                >
                  <AppIcon aria-hidden className="h-4 w-4" name="content-copy" />
                </Button>
              </Tooltip>
            </div>
            <pre
              className={
                includeTimings
                  ? "mt-1 max-h-32 overflow-auto whitespace-pre-wrap break-words font-mono text-xs leading-5 text-base-content/85"
                  : "min-w-0 flex-1 whitespace-pre-wrap break-words font-mono text-xs leading-5 text-base-content/85"
              }
            >
              {errorMessage}
            </pre>
          </div>
        ) : null}
      </div>
    </details>
  );
}

function hasAttemptEvidence(attempt: ApiPoolUpstreamRequestAttempt, includeTimings: boolean) {
  return Boolean(
    includeTimings ||
      attempt.failureKind ||
      attempt.errorMessage?.trim() ||
      (attempt.downstreamHttpStatus != null && attempt.downstreamHttpStatus !== attempt.httpStatus),
  );
}

function AttemptError({ attempt }: { attempt: ApiPoolUpstreamRequestAttempt }) {
  const message = attempt.errorMessage?.trim() || "";
  return (
    <div className="min-w-0">
      <p className="line-clamp-2 break-words text-xs leading-5 text-base-content/75">
        {attempt.failureKind ? (
          <span className="font-mono text-base-content/85">{attempt.failureKind}</span>
        ) : null}
        {attempt.failureKind && message ? <span className="text-base-content/45"> · </span> : null}
        {message || (attempt.failureKind ? null : "-")}
      </p>
    </div>
  );
}

function AttemptTime({
  attempt,
  dateFormatter,
  timeFormatter,
}: {
  attempt: ApiPoolUpstreamRequestAttempt;
  dateFormatter: Intl.DateTimeFormat;
  timeFormatter: Intl.DateTimeFormat;
}) {
  const time = new Date(attempt.occurredAt);
  if (Number.isNaN(time.valueOf())) return <>{attempt.occurredAt}</>;
  return (
    <time dateTime={attempt.occurredAt}>
      <span className="block whitespace-nowrap">{dateFormatter.format(time)}</span>
      <span className="block whitespace-nowrap">{timeFormatter.format(time)}</span>
    </time>
  );
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
  const dateFormatter = useMemo(
    () => new Intl.DateTimeFormat(localeTag, { month: "2-digit", day: "2-digit" }),
    [localeTag],
  );
  const timeFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(localeTag, {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
        hour12: false,
      }),
    [localeTag],
  );
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
        if (requestSeq === requestSeqRef.current)
          setError(requestError instanceof Error ? requestError.message : String(requestError));
      })
      .finally(() => {
        if (requestSeq === requestSeqRef.current) setLoading(false);
      });
  };

  if (loading && !response)
    return (
      <div className="flex justify-center py-10">
        <Spinner />
      </div>
    );
  if (error) return <Alert variant="warning">{error}</Alert>;
  if (!response || response.items.length === 0)
    return (
      <p className="text-sm text-base-content/68">{t("accountPool.upstreamAttempts.empty")}</p>
    );

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
      <div className="hidden overflow-x-auto rounded-lg border border-base-300/70 bg-base-100/65 pb-1 pl-6 pr-1 pt-1 md:block">
        <table
          className="min-w-full w-max border-collapse text-left text-sm"
          data-testid="upstream-account-call-records-table"
        >
          <thead className="border-b border-base-300/70 bg-base-200/55 text-xs font-medium text-base-content/65">
            <tr>
              <th className="min-w-24 px-3 py-2.5">
                {t("accountPool.upstreamAttempts.columns.time")}
              </th>
              <th className="whitespace-nowrap px-3 py-2.5">
                {t("accountPool.upstreamAttempts.columns.call")}
              </th>
              <th className="min-w-56 px-3 py-2.5">
                {t("accountPool.upstreamAttempts.columns.model")}
              </th>
              <th className="w-40 px-3 py-2.5">
                {t("accountPool.upstreamAttempts.columns.result")}
              </th>
              <th className="w-40 px-3 py-2.5">
                {t("accountPool.upstreamAttempts.columns.proxy")}
              </th>
              <th className="w-48 px-3 py-2.5">
                {t("accountPool.upstreamAttempts.columns.timing")}
              </th>
              <th className="min-w-64 px-3 py-2.5">
                {t("accountPool.upstreamAttempts.columns.error")}
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-base-300/70">
            {response.items.map((attempt) => {
              const proxy = formatProxyBinding(attempt, proxyBindingNodesByKey, t);
              return (
                <Fragment key={attempt.attemptId}>
                  <tr
                    className={
                      attempt.attemptId === focusedAttemptId ? "bg-info/10" : "hover:bg-base-200/35"
                    }
                    data-attempt-id={attempt.attemptId}
                    data-testid={`account-attempt-record-${attempt.attemptId}`}
                  >
                    <td className="min-w-24 px-3 py-3 align-top font-mono text-xs tabular-nums text-base-content/70">
                      <AttemptTime
                        attempt={attempt}
                        dateFormatter={dateFormatter}
                        timeFormatter={timeFormatter}
                      />
                    </td>
                    <td className="whitespace-nowrap px-3 py-3 align-top font-mono text-xs">
                      <Link
                        className="inline-block whitespace-nowrap text-info underline underline-offset-4 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                        title={attempt.attemptId}
                        to={`/records?attemptId=${encodeURIComponent(attempt.attemptId)}&rangePreset=7d`}
                      >
                        {attempt.attemptId}
                      </Link>
                      <div className="mt-1 text-[11px] text-base-content/55">
                        {attempt.invokeId}
                      </div>
                    </td>
                    <td className="px-3 py-3 align-top">
                      <ModelMapping attempt={attempt} t={t} />
                    </td>
                    <td className="px-3 py-3 align-top">
                      <AttemptResult attempt={attempt} t={t} />
                    </td>
                    <td className="px-3 py-3 align-top">
                      <span
                        className={
                          proxy.resolved
                            ? "block max-w-40 truncate font-medium"
                            : "block max-w-40 truncate font-mono text-xs"
                        }
                        title={proxy.title}
                      >
                        {proxy.value}
                      </span>
                    </td>
                    <td className="px-3 py-3 align-top font-mono text-xs tabular-nums text-base-content/75">
                      <AttemptTiming attempt={attempt} t={t} />
                    </td>
                    <td className="px-3 py-3 align-top">
                      <AttemptError attempt={attempt} />
                    </td>
                  </tr>
                  {hasAttemptEvidence(attempt, false) ? (
                    <tr className={attempt.attemptId === focusedAttemptId ? "bg-info/10" : ""}>
                      <td className="px-3 pb-3 pt-0" colSpan={7}>
                        <AttemptEvidenceDisclosure
                          attempt={attempt}
                          includeTimings={false}
                          isFocused={attempt.attemptId === focusedAttemptId}
                          proxy={proxy}
                          t={t}
                        />
                      </td>
                    </tr>
                  ) : null}
                </Fragment>
              );
            })}
          </tbody>
        </table>
      </div>
      <div className="overflow-x-auto rounded-lg border border-base-300/70 bg-base-100/65 md:hidden">
        <table
          className="min-w-[560px] w-max border-collapse text-left text-xs"
          data-testid="upstream-account-call-records-mobile-table"
        >
          <thead className="border-b border-base-300/70 bg-base-200/55 text-base-content/65">
            <tr>
              <th className="w-16 px-2 py-2">{t("accountPool.upstreamAttempts.columns.time")}</th>
              <th className="px-2 py-2">{t("accountPool.upstreamAttempts.columns.call")}</th>
              <th className="w-20 px-2 py-2">{t("accountPool.upstreamAttempts.columns.result")}</th>
              <th className="px-2 py-2">{t("accountPool.upstreamAttempts.columns.error")}</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-base-300/70">
            {response.items.map((attempt) => {
              const proxy = formatProxyBinding(attempt, proxyBindingNodesByKey, t);
              return (
                <Fragment key={attempt.attemptId}>
                  <tr
                    className={attempt.attemptId === focusedAttemptId ? "bg-info/10" : ""}
                    data-attempt-id={attempt.attemptId}
                  >
                    <td className="px-2 py-2 align-top font-mono tabular-nums text-base-content/70">
                      <AttemptTime
                        attempt={attempt}
                        dateFormatter={dateFormatter}
                        timeFormatter={timeFormatter}
                      />
                    </td>
                    <td className="px-2 py-2 align-top">
                      <Link
                        className="block whitespace-nowrap font-mono text-info underline underline-offset-4"
                        title={attempt.attemptId}
                        to={`/records?attemptId=${encodeURIComponent(attempt.attemptId)}&rangePreset=7d`}
                      >
                        {attempt.attemptId}
                      </Link>
                      <div className="mt-1 font-mono text-[11px] text-base-content/55">
                        {attempt.invokeId}
                      </div>
                      <div className="mt-1">
                        <ModelMapping attempt={attempt} t={t} />
                      </div>
                    </td>
                    <td className="px-2 py-2 align-top">
                      <AttemptResult attempt={attempt} t={t} />
                    </td>
                    <td className="px-2 py-2 align-top">
                      <AttemptError attempt={attempt} />
                    </td>
                  </tr>
                  {hasAttemptEvidence(attempt, true) ? (
                    <tr className={attempt.attemptId === focusedAttemptId ? "bg-info/10" : ""}>
                      <td className="px-2 pb-2 pt-0" colSpan={4}>
                        <AttemptEvidenceDisclosure
                          attempt={attempt}
                          includeTimings
                          isFocused={attempt.attemptId === focusedAttemptId}
                          proxy={proxy}
                          t={t}
                        />
                      </td>
                    </tr>
                  ) : null}
                </Fragment>
              );
            })}
          </tbody>
        </table>
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
