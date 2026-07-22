import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { Button } from "../../components/ui/button";
import {
  FilterableCombobox,
  type FilterableComboboxOption,
} from "../../components/ui/filterable-combobox";
import { SelectField, type SelectFieldOption } from "../../components/ui/select-field";
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
import { normalizeModelComparisonKey } from "../../lib/invocation";
import { InvocationWorkflowAttemptRecord } from "../invocations/InvocationWorkflowDetailPanel";

const PAGE_SIZE = 50;
const CALL_SHORT_ID_PATTERN = /^[ABCDEFGHJKMNPQRSTUVWXYZ23456789]{10}$/;
const UNBOUND_STICKY_KEY = "__unbound__";
const FILTER_INPUT_CLASS_NAME =
  "h-9 w-full rounded-md border border-base-300/80 bg-base-100 px-3 text-sm text-base-content shadow-sm outline-none transition focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100 disabled:cursor-not-allowed disabled:opacity-60";

type UpstreamAttemptTypeFilter = "" | "normal" | "remote_v2" | "compact" | "image";

type AttemptFilterState = {
  type: UpstreamAttemptTypeFilter;
  model: string;
  stickyKey: string;
};

const DEFAULT_FILTERS: AttemptFilterState = {
  type: "",
  model: "",
  stickyKey: "",
};

function createDefaultFilters(): AttemptFilterState {
  return { ...DEFAULT_FILTERS };
}

function normalizeFilterValue(value: string | null | undefined) {
  return value?.trim() ?? "";
}

function buildAttemptTypeOptions(t: (key: string) => string): SelectFieldOption[] {
  return [
    {
      value: "",
      label: t("accountPool.upstreamAttempts.filters.typeAll"),
    },
    {
      value: "normal",
      label: t("table.endpoint.responsesBadge"),
    },
    {
      value: "remote_v2",
      label: t("table.endpoint.remoteV2Badge"),
    },
    {
      value: "compact",
      label: t("table.endpoint.compactBadge"),
    },
    {
      value: "image",
      label: t("table.endpoint.imageBadge"),
    },
  ];
}

function buildModelOptions(
  items: ApiPoolUpstreamRequestAttempt[] | undefined,
  selectedValue: string,
  allLabel: string,
): FilterableComboboxOption[] {
  const optionMap = new Map<string, { value: string; latestCreatedAt: string }>();
  for (const item of items ?? []) {
    for (const candidate of [item.requestModel, item.responseModel, item.model]) {
      const displayValue = normalizeFilterValue(candidate);
      if (!displayValue) continue;
      const key = normalizeModelComparisonKey(displayValue) ?? displayValue.toLowerCase();
      const existing = optionMap.get(key);
      if (!existing || item.createdAt > existing.latestCreatedAt) {
        optionMap.set(key, {
          value: displayValue,
          latestCreatedAt: item.createdAt,
        });
      }
    }
  }
  const trimmedSelectedValue = normalizeFilterValue(selectedValue);
  if (trimmedSelectedValue) {
    const selectedKey =
      normalizeModelComparisonKey(trimmedSelectedValue) ?? trimmedSelectedValue.toLowerCase();
    if (!optionMap.has(selectedKey)) {
      optionMap.set(selectedKey, {
        value: trimmedSelectedValue,
        latestCreatedAt: "",
      });
    }
  }
  return [
    {
      value: "",
      label: allLabel,
    },
    ...Array.from(optionMap.values())
      .sort(
        (left, right) =>
          right.latestCreatedAt.localeCompare(left.latestCreatedAt) ||
          left.value.localeCompare(right.value),
      )
      .map((option) => ({
        value: option.value,
        label: option.value,
      })),
  ];
}

function buildStickyKeyOptions(
  stickyKeyOptions: UpstreamAccountAttemptListResponse["stickyKeyOptions"] | undefined,
  selectedValue: string,
  allLabel: string,
  unboundLabel: string,
): SelectFieldOption[] {
  const options = new Map<string, SelectFieldOption>();
  for (const option of stickyKeyOptions ?? []) {
    options.set(option.value, {
      value: option.value,
      label: option.value === UNBOUND_STICKY_KEY ? unboundLabel : option.value,
    });
  }
  const trimmedSelectedValue = normalizeFilterValue(selectedValue);
  if (trimmedSelectedValue && !options.has(trimmedSelectedValue)) {
    options.set(trimmedSelectedValue, {
      value: trimmedSelectedValue,
      label: trimmedSelectedValue === UNBOUND_STICKY_KEY ? unboundLabel : trimmedSelectedValue,
    });
  }
  return [
    {
      value: "",
      label: allLabel,
    },
    ...Array.from(options.values()),
  ];
}

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
    compactionRequestKind: attempt.compactionRequestKind ?? undefined,
    compactionResponseKind: attempt.compactionResponseKind ?? undefined,
    imageIntent: attempt.imageIntent ?? undefined,
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
    compactionRequestKind: attempt.compactionRequestKind ?? null,
    imageIntent: attempt.imageIntent ?? null,
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
    compactionResponseKind: attempt.compactionResponseKind ?? null,
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

function resolveInvocationRecord(attempt: ApiPoolUpstreamRequestAttempt): ApiInvocation {
  return attempt.invocationRecord ?? buildSyntheticInvocationRecord(attempt);
}

function resolveWorkflowAttemptEntry(
  attempt: ApiPoolUpstreamRequestAttempt,
  proxyDisplay: ReturnType<typeof formatProxyBinding>,
): ApiInvocationWorkflowTimelineEntry {
  return attempt.workflowEntry ?? buildSyntheticWorkflowAttemptEntry(attempt, proxyDisplay);
}

export function UpstreamAccountAttemptTimeline({
  accountId,
  focusedAttemptId,
  focusVersion = 0,
  interactionBoundary = null,
  onFocusRequestHandled,
}: {
  accountId: number;
  focusedAttemptId: string | null;
  focusVersion?: number;
  interactionBoundary?: HTMLElement | null;
  onFocusRequestHandled?: (version: number) => void;
}) {
  const { t, locale } = useTranslation();
  const [response, setResponse] = useState<UpstreamAccountAttemptListResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filters, setFilters] = useState<AttemptFilterState>(() => createDefaultFilters());
  const [activeFocus, setActiveFocus] = useState<{
    attemptId: string;
    version: number;
  } | null>(null);
  const requestSeqRef = useRef(0);
  const resolvedAccountIdRef = useRef<number | null>(null);
  const focusDismissTimerRef = useRef<number | null>(null);
  const attemptElementMapRef = useRef(new Map<string, HTMLDivElement>());
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
  const typeOptions = useMemo(() => buildAttemptTypeOptions(t), [t]);
  const modelOptions = useMemo(
    () =>
      buildModelOptions(
        response?.items,
        filters.model,
        t("accountPool.upstreamAttempts.filters.modelAll"),
      ),
    [filters.model, response?.items, t],
  );
  const stickyKeyOptions = useMemo(
    () =>
      buildStickyKeyOptions(
        response?.stickyKeyOptions,
        filters.stickyKey,
        t("accountPool.upstreamAttempts.filters.conversationAll"),
        t("accountPool.upstreamAttempts.filters.conversationUnbound"),
      ),
    [filters.stickyKey, response?.stickyKeyOptions, t],
  );

  const clearFocusDismissTimer = useCallback(() => {
    if (focusDismissTimerRef.current == null) return;
    window.clearTimeout(focusDismissTimerRef.current);
    focusDismissTimerRef.current = null;
  }, []);

  const setAttemptElement = useCallback((attemptId: string, node: HTMLDivElement | null) => {
    if (node) {
      attemptElementMapRef.current.set(attemptId, node);
      return;
    }
    attemptElementMapRef.current.delete(attemptId);
  }, []);

  useEffect(() => {
    return () => {
      clearFocusDismissTimer();
    };
  }, [clearFocusDismissTimer]);

  const loadAttemptsPage = useCallback(
    (page: number, nextFilters: AttemptFilterState, signal?: AbortSignal) => {
      const requestSeq = requestSeqRef.current + 1;
      requestSeqRef.current = requestSeq;
      clearFocusDismissTimer();
      setLoading(true);
      setError(null);
      setResponse(null);
      setActiveFocus(null);
      void fetchUpstreamAccountAttempts(accountId, {
        type: nextFilters.type || undefined,
        model: nextFilters.model || undefined,
        stickyKey: nextFilters.stickyKey || undefined,
        page,
        pageSize: PAGE_SIZE,
        signal,
      })
        .then((next) => {
          if (signal?.aborted || requestSeq !== requestSeqRef.current) return;
          resolvedAccountIdRef.current = accountId;
          setResponse(next);
        })
        .catch((requestError) => {
          if (signal?.aborted || requestSeq !== requestSeqRef.current) return;
          resolvedAccountIdRef.current = accountId;
          setResponse(null);
          setError(requestError instanceof Error ? requestError.message : String(requestError));
        })
        .finally(() => {
          if (!signal?.aborted && requestSeq === requestSeqRef.current) setLoading(false);
        });
    },
    [accountId, clearFocusDismissTimer],
  );

  const updateFilters = useCallback((patch: Partial<AttemptFilterState>) => {
    setFilters((current) => {
      const next = {
        ...current,
        ...patch,
      };
      next.model = normalizeFilterValue(next.model);
      next.stickyKey = normalizeFilterValue(next.stickyKey);
      return next.type === current.type &&
        next.model === current.model &&
        next.stickyKey === current.stickyKey
        ? current
        : next;
    });
  }, []);

  useEffect(() => {
    if (focusedAttemptId != null) return;
    const controller = new AbortController();
    loadAttemptsPage(1, filters, controller.signal);
    return () => controller.abort();
  }, [filters, focusedAttemptId, loadAttemptsPage]);

  useEffect(() => {
    if (focusedAttemptId == null) return;
    const controller = new AbortController();
    const requestSeq = requestSeqRef.current + 1;
    requestSeqRef.current = requestSeq;
    setFilters(createDefaultFilters());
    setLoading(true);
    setError(null);
    setResponse(null);
    setActiveFocus(null);
    clearFocusDismissTimer();
    void locateUpstreamAccountAttempt(accountId, focusedAttemptId, {
      pageSize: PAGE_SIZE,
      signal: controller.signal,
    })
      .then((next) => {
        if (controller.signal.aborted || requestSeq !== requestSeqRef.current) return;
        resolvedAccountIdRef.current = accountId;
        setResponse(next);
        setActiveFocus({
          attemptId: focusedAttemptId,
          version: focusVersion,
        });
      })
      .catch((requestError) => {
        if (controller.signal.aborted || requestSeq !== requestSeqRef.current) return;
        resolvedAccountIdRef.current = accountId;
        setResponse(null);
        setActiveFocus(null);
        setError(
          requestError instanceof Error && requestError.message.includes("404")
            ? t("accountPool.upstreamAttempts.locateUnavailable")
            : requestError instanceof Error
              ? requestError.message
              : String(requestError),
        );
      })
      .finally(() => {
        if (controller.signal.aborted || requestSeq !== requestSeqRef.current) return;
        onFocusRequestHandled?.(focusVersion);
        setLoading(false);
      });
    return () => controller.abort();
  }, [accountId, clearFocusDismissTimer, focusVersion, focusedAttemptId, onFocusRequestHandled, t]);

  useLayoutEffect(() => {
    if (!activeFocus) return;
    const target = attemptElementMapRef.current.get(activeFocus.attemptId);
    if (!target) return;
    target.scrollIntoView({
      behavior: "smooth",
      block: "nearest",
    });
  }, [activeFocus]);

  useEffect(() => {
    if (!activeFocus || !interactionBoundary) return;
    clearFocusDismissTimer();
    const dismissFocusedAttempt = () => {
      if (focusDismissTimerRef.current != null) return;
      focusDismissTimerRef.current = window.setTimeout(() => {
        focusDismissTimerRef.current = null;
        setActiveFocus((current) =>
          current?.attemptId === activeFocus.attemptId && current.version === activeFocus.version
            ? null
            : current,
        );
      }, 1_500);
    };
    interactionBoundary.addEventListener("pointerdown", dismissFocusedAttempt, {
      once: true,
      passive: true,
    });
    interactionBoundary.addEventListener("keydown", dismissFocusedAttempt, {
      once: true,
    });
    return () => {
      interactionBoundary.removeEventListener("pointerdown", dismissFocusedAttempt);
      interactionBoundary.removeEventListener("keydown", dismissFocusedAttempt);
      clearFocusDismissTimer();
    };
  }, [activeFocus, clearFocusDismissTimer, interactionBoundary]);

  const loadPage = (page: number) => loadAttemptsPage(page, filters);
  const showListLoading = loading && !response;
  const showListError = !loading && error != null;
  const showListEmpty = !loading && !error && (!response || response.items.length === 0);

  return (
    <section className="space-y-3" data-testid="upstream-account-call-records">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <p className="text-sm text-base-content/68">
          {t("accountPool.upstreamAttempts.description")}
        </p>
        {response ? (
          <span className="shrink-0 text-sm tabular-nums text-base-content/58">
            {t("accountPool.upstreamAttempts.total", { count: response.total })}
          </span>
        ) : null}
      </div>
      <div
        className="grid gap-3 rounded-2xl border border-base-300/70 bg-base-100/55 p-3 sm:grid-cols-[minmax(9rem,0.8fr)_minmax(12rem,1fr)_minmax(12rem,1fr)]"
        data-testid="upstream-account-attempt-filter-bar"
      >
        <SelectField
          label={t("accountPool.upstreamAttempts.filters.type")}
          value={filters.type}
          onValueChange={(value) =>
            updateFilters({ type: (value || "") as UpstreamAttemptTypeFilter })
          }
          options={typeOptions}
          size="sm"
          data-testid="upstream-attempt-type-filter"
        />
        <label className="field">
          <span className="field-label">{t("accountPool.upstreamAttempts.filters.model")}</span>
          <FilterableCombobox
            label={t("accountPool.upstreamAttempts.filters.model")}
            id="upstream-attempt-model-filter"
            value={filters.model}
            onValueChange={(value) => updateFilters({ model: value })}
            options={modelOptions}
            placeholder={t("accountPool.upstreamAttempts.filters.modelAll")}
            emptyText={t("accountPool.upstreamAttempts.empty")}
            inputClassName={FILTER_INPUT_CLASS_NAME}
          />
        </label>
        <SelectField
          label={t("accountPool.upstreamAttempts.filters.conversation")}
          value={filters.stickyKey}
          onValueChange={(value) => updateFilters({ stickyKey: value || "" })}
          options={stickyKeyOptions}
          size="sm"
          data-testid="upstream-attempt-conversation-filter"
        />
      </div>
      <div className="space-y-3" data-testid="upstream-account-attempt-list">
        {showListLoading ? (
          <div className="flex justify-center rounded-2xl border border-base-300/60 bg-base-100/45 py-10">
            <Spinner />
          </div>
        ) : null}
        {showListError ? <Alert variant="warning">{error}</Alert> : null}
        {showListEmpty ? (
          <p className="rounded-2xl border border-base-300/60 bg-base-100/45 px-4 py-6 text-sm text-base-content/68">
            {t("accountPool.upstreamAttempts.empty")}
          </p>
        ) : null}
        {!showListLoading && !showListError
          ? response?.items.map((attempt) => {
              const proxyDisplay = formatProxyBinding(
                attempt,
                proxyBindingNodesByKey,
                proxyDirectLabel,
              );
              const callShortId = displayCallShortId(attempt.invokeId);
              const workflowEntry = resolveWorkflowAttemptEntry(attempt, proxyDisplay);
              const invocationRecord = resolveInvocationRecord(attempt);
              const isFocused = attempt.attemptId === activeFocus?.attemptId;

              return (
                <InvocationWorkflowAttemptRecord
                  key={attempt.attemptId}
                  containerRef={(node) => setAttemptElement(attempt.attemptId, node)}
                  record={invocationRecord}
                  entry={workflowEntry}
                  localeTag={localeTag}
                  isZh={isZh}
                  summaryIdentity={callShortId ?? attempt.attemptId}
                  focused={isFocused}
                  focusVersion={isFocused ? (activeFocus?.version ?? 0) : 0}
                  defaultSection={isFocused ? "timing" : null}
                  testId={`account-attempt-record-${attempt.attemptId}`}
                />
              );
            })
          : null}
      </div>
      {response && response.total > 0 ? (
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
      ) : null}
    </section>
  );
}
