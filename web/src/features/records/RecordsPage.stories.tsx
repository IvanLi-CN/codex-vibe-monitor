import type { Meta, StoryObj } from "@storybook/react-vite";
import { type ReactNode, useEffect, useMemo, useRef } from "react";
import { MemoryRouter } from "react-router-dom";
import { expect, userEvent, waitFor, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type {
  ApiInvocation,
  InvocationRecordsQuery,
  InvocationSortBy,
  InvocationSortOrder,
} from "../../lib/api";
import RecordsPage from "../../pages/Records";
import {
  createStoryInvocationRecordDetailsById,
  createStoryInvocationRecordsResponse,
  createStoryInvocationRecordsSummary,
  createStoryInvocationResponseBodiesById,
  createStoryPoolAttemptsByInvokeId,
  STORYBOOK_INVOCATION_RECORDS,
  summarizeInvocationRecords,
} from "./invocationRecordsStoryFixtures";

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 text-base-content">
      <div className="app-shell-boundary px-4 py-6">{children}</div>
    </div>
  );
}

const SNAPSHOT_ID = 8844;
const FAST_POLL_MS = 30;

function alignStoryRecordsToNow(records: ApiInvocation[]) {
  if (records.length === 0) return [];

  const latestOccurredAt = Math.max(...records.map((record) => Date.parse(record.occurredAt)));
  const anchor = new Date();
  anchor.setSeconds(0, 0);

  return records.map((record) => {
    const occurredDeltaMs = latestOccurredAt - Date.parse(record.occurredAt);
    const createdDeltaMs = latestOccurredAt - Date.parse(record.createdAt);
    return {
      ...record,
      occurredAt: new Date(anchor.getTime() - occurredDeltaMs).toISOString(),
      createdAt: new Date(anchor.getTime() - createdDeltaMs).toISOString(),
    };
  });
}

const STORYBOOK_RECENT_INVOCATION_RECORDS = alignStoryRecordsToNow(STORYBOOK_INVOCATION_RECORDS);
const STORYBOOK_POOL_DETAILS_RECORDS = STORYBOOK_RECENT_INVOCATION_RECORDS.filter(
  (record) => record.routeMode === "pool" && record.status !== "running",
);

function normalizeText(value: string | null) {
  const normalized = value?.trim() ?? "";
  return normalized.toLowerCase();
}

function matchesText(value: string | undefined, query: string | null) {
  const normalizedQuery = normalizeText(query ?? null);
  if (!normalizedQuery) return true;
  return normalizeText(value ?? null) === normalizedQuery;
}

function parseListParam(value: string | null) {
  const deduped = new Set<string>();
  for (const item of (value ?? "").split(",")) {
    const normalized = normalizeText(item);
    if (!normalized) continue;
    deduped.add(normalized);
  }
  return Array.from(deduped);
}

function resolveModelValue(record: ApiInvocation, target: "request" | "response") {
  if (target === "response") {
    return normalizeText(record.responseModel ?? record.model ?? null);
  }
  return normalizeText(record.requestModel ?? record.model ?? null);
}

function hasModelReroute(record: ApiInvocation) {
  const requestModel = normalizeText(record.requestModel ?? null);
  const responseModel = normalizeText(record.responseModel ?? null);
  return !!requestModel && !!responseModel && requestModel !== responseModel;
}

function parseModelReroutedFilter(value: string | null) {
  const normalized = normalizeText(value);
  if (normalized === "true") return true;
  if (normalized === "false") return false;
  return null;
}

function matchesNumberRange(
  value: number | null | undefined,
  minRaw: string | null,
  maxRaw: string | null,
) {
  const min = minRaw ? Number(minRaw) : null;
  const max = maxRaw ? Number(maxRaw) : null;
  const numericValue = typeof value === "number" && Number.isFinite(value) ? value : null;
  if (min != null && (numericValue == null || numericValue < min)) return false;
  if (max != null && (numericValue == null || numericValue > max)) return false;
  return true;
}

function resolveAttemptInvokeId(
  attemptsByInvokeId: Record<string, Array<{ attemptId: string }>>,
  attemptIdRaw: string | null,
) {
  const attemptId = normalizeText(attemptIdRaw);
  if (!attemptId) return null;
  for (const [invokeId, attempts] of Object.entries(attemptsByInvokeId)) {
    if (attempts.some((attempt) => normalizeText(attempt.attemptId) === attemptId)) {
      return invokeId;
    }
  }
  return "__missing_attempt__";
}

function filterRecords(
  records: ApiInvocation[],
  params: URLSearchParams,
  attemptsByInvokeId: Record<string, Array<{ attemptId: string }>> = {},
) {
  const from = params.get("from");
  const to = params.get("to");
  const keyword = normalizeText(params.get("keyword"));
  const invokeId = normalizeText(params.get("invokeId") ?? params.get("requestId"));
  const attemptInvokeId = resolveAttemptInvokeId(attemptsByInvokeId, params.get("attemptId"));
  const modelValues = parseListParam(params.get("models"));
  const reasoningEffortValues = parseListParam(params.get("reasoningEfforts"));
  const modelTarget =
    normalizeText(params.get("modelTarget")) === "response" ? "response" : "request";
  const modelRerouted = parseModelReroutedFilter(params.get("modelRerouted"));

  return records.filter((record) => {
    const occurredAt = Date.parse(record.occurredAt);
    if (from && occurredAt < Date.parse(from)) return false;
    if (to && occurredAt >= Date.parse(to)) return false;
    if (!matchesText(record.status, params.get("status"))) return false;
    if (modelValues.length > 0) {
      const modelValue = resolveModelValue(record, modelTarget);
      if (!modelValue || !modelValues.includes(modelValue)) return false;
    } else if (!matchesText(record.model, params.get("model"))) {
      return false;
    }
    if (modelRerouted === true && !hasModelReroute(record)) return false;
    if (modelRerouted === false && hasModelReroute(record)) return false;
    if (!matchesText(record.endpoint, params.get("endpoint"))) return false;
    if (!matchesText(record.failureClass ?? undefined, params.get("failureClass"))) return false;
    if (!matchesText(record.failureKind, params.get("failureKind"))) return false;
    if (!matchesText(record.promptCacheKey, params.get("promptCacheKey"))) return false;
    const upstreamScope = normalizeText(record.upstreamScope ?? null)
      ? record.upstreamScope
      : record.routeMode === "pool"
        ? "internal"
        : "external";
    if (!matchesText(upstreamScope ?? undefined, params.get("upstreamScope"))) return false;
    const upstreamAccountId = params.get("upstreamAccountId");
    if (
      upstreamAccountId &&
      (!Number.isFinite(Number(upstreamAccountId)) ||
        record.upstreamAccountId !== Number(upstreamAccountId))
    ) {
      return false;
    }
    if (!matchesText(record.proxyDisplayName, params.get("proxyDisplayName"))) return false;
    if (!matchesText(record.transport ?? undefined, params.get("transport"))) return false;
    if (!matchesText(record.serviceTier, params.get("serviceTier"))) return false;
    if (reasoningEffortValues.length > 0) {
      const reasoningEffort = normalizeText(record.reasoningEffort ?? null);
      if (!reasoningEffort || !reasoningEffortValues.includes(reasoningEffort)) return false;
    } else if (!matchesText(record.reasoningEffort, params.get("reasoningEffort"))) {
      return false;
    }
    if (!matchesText(record.requesterIp, params.get("requesterIp"))) return false;
    if (
      !matchesNumberRange(
        record.totalTokens,
        params.get("minTotalTokens"),
        params.get("maxTotalTokens"),
      )
    )
      return false;
    if (!matchesNumberRange(record.tTotalMs, params.get("minTotalMs"), params.get("maxTotalMs")))
      return false;
    if (invokeId && normalizeText(record.invokeId) !== invokeId) return false;
    if (attemptInvokeId && normalizeText(record.invokeId) !== normalizeText(attemptInvokeId)) {
      return false;
    }

    if (!keyword) return true;
    const haystack = [
      record.invokeId,
      record.model,
      record.endpoint,
      record.errorMessage,
      record.failureKind,
      record.proxyDisplayName,
      record.requesterIp,
    ]
      .join(" ")
      .toLowerCase();
    return haystack.includes(keyword);
  });
}

function compareNumbers(left?: number | null, right?: number | null) {
  const leftValue =
    typeof left === "number" && Number.isFinite(left) ? left : Number.NEGATIVE_INFINITY;
  const rightValue =
    typeof right === "number" && Number.isFinite(right) ? right : Number.NEGATIVE_INFINITY;
  return leftValue - rightValue;
}

function sortRecords(
  records: ApiInvocation[],
  sortBy: InvocationSortBy,
  sortOrder: InvocationSortOrder,
) {
  const direction = sortOrder === "asc" ? 1 : -1;
  return [...records].sort((left, right) => {
    let result = 0;
    switch (sortBy) {
      case "totalTokens":
        result = compareNumbers(left.totalTokens, right.totalTokens);
        break;
      case "cost":
        result = compareNumbers(left.cost, right.cost);
        break;
      case "tTotalMs":
        result = compareNumbers(left.tTotalMs, right.tTotalMs);
        break;
      case "tUpstreamTtfbMs":
        result = compareNumbers(left.tUpstreamTtfbMs, right.tUpstreamTtfbMs);
        break;
      case "status":
        result = (left.status ?? "").localeCompare(right.status ?? "");
        break;
      default:
        result = Date.parse(left.occurredAt) - Date.parse(right.occurredAt);
        break;
    }
    if (result !== 0) return result * direction;
    return (right.id - left.id) * direction;
  });
}

function paginateRecords(records: ApiInvocation[], query: InvocationRecordsQuery) {
  const page = Math.max(1, Number(query.page ?? 1));
  const pageSize = Math.max(1, Number(query.pageSize ?? 20));
  const start = (page - 1) * pageSize;
  const paged = records.slice(start, start + pageSize);
  return { page, pageSize, paged };
}

function buildSuggestionBucket(
  records: ApiInvocation[],
  extract: (record: ApiInvocation) => string | null | undefined,
  query?: string | null,
) {
  const counts = new Map<string, number>();
  const normalizedQuery = normalizeText(query ?? null);
  for (const record of records) {
    const rawValue = extract(record);
    const value = rawValue?.trim() ?? "";
    if (!value) continue;
    if (normalizedQuery && !value.toLowerCase().includes(normalizedQuery)) continue;
    counts.set(value, (counts.get(value) ?? 0) + 1);
  }

  const sorted = Array.from(counts.entries())
    .map(([value, count]) => ({ value, count }))
    .sort((left, right) => {
      if (right.count !== left.count) return right.count - left.count;
      return left.value.localeCompare(right.value);
    });

  const limit = 30;
  return {
    items: sorted.slice(0, limit),
    hasMore: sorted.length > limit,
  };
}

function buildUpstreamAccountBucket(records: ApiInvocation[], query?: string | null) {
  const normalizedQuery = normalizeText(query ?? null);
  const counts = new Map<number, { count: number; label: string }>();
  for (const record of records) {
    const accountId = record.upstreamAccountId;
    if (typeof accountId !== "number" || !Number.isFinite(accountId)) continue;
    const label = `${record.upstreamAccountName?.trim() || `#${accountId}`} (#${accountId})`;
    const searchText = `${record.upstreamAccountName ?? ""} ${accountId}`.toLowerCase();
    if (normalizedQuery && !searchText.includes(normalizedQuery)) continue;
    const current = counts.get(accountId);
    counts.set(accountId, {
      count: (current?.count ?? 0) + 1,
      label,
    });
  }

  const items = Array.from(counts.entries())
    .map(([value, item]) => ({
      value: String(value),
      label: item.label,
      count: item.count,
    }))
    .sort((left, right) => {
      if (right.count !== left.count) return right.count - left.count;
      return left.label.localeCompare(right.label);
    });
  const limit = 30;
  return {
    items: items.slice(0, limit),
    hasMore: items.length > limit,
  };
}

function suggestionFieldQuery(params: URLSearchParams, field: string) {
  return params.get("suggestField") === field ? params.get("suggestQuery") : null;
}

function buildSuggestions(records: ApiInvocation[], params: URLSearchParams) {
  return {
    model: buildSuggestionBucket(
      records,
      (record) => record.model,
      suggestionFieldQuery(params, "model"),
    ),
    requestModel: buildSuggestionBucket(
      records,
      (record) => record.requestModel ?? record.model,
      suggestionFieldQuery(params, "requestModel"),
    ),
    responseModel: buildSuggestionBucket(
      records,
      (record) => record.responseModel ?? record.model,
      suggestionFieldQuery(params, "responseModel"),
    ),
    endpoint: buildSuggestionBucket(
      records,
      (record) => record.endpoint,
      suggestionFieldQuery(params, "endpoint"),
    ),
    failureKind: buildSuggestionBucket(
      records,
      (record) => record.failureKind,
      suggestionFieldQuery(params, "failureKind"),
    ),
    stickyKey: { items: [], hasMore: false },
    promptCacheKey: buildSuggestionBucket(
      records,
      (record) => record.promptCacheKey,
      suggestionFieldQuery(params, "promptCacheKey"),
    ),
    requesterIp: buildSuggestionBucket(
      records,
      (record) => record.requesterIp,
      suggestionFieldQuery(params, "requesterIp"),
    ),
    proxyDisplayName: buildSuggestionBucket(
      records,
      (record) => record.proxyDisplayName,
      suggestionFieldQuery(params, "proxyDisplayName"),
    ),
    upstreamAccount: buildUpstreamAccountBucket(
      records,
      suggestionFieldQuery(params, "upstreamAccount"),
    ),
    serviceTier: buildSuggestionBucket(
      records,
      (record) => record.serviceTier,
      suggestionFieldQuery(params, "serviceTier"),
    ),
    reasoningEffort: buildSuggestionBucket(
      records,
      (record) => record.reasoningEffort,
      suggestionFieldQuery(params, "reasoningEffort"),
    ),
  };
}

function filterSuggestionSourceRecords(
  records: ApiInvocation[],
  params: URLSearchParams,
  attemptsByInvokeId: Record<string, Array<{ attemptId: string }>> = {},
) {
  const nextParams = new URLSearchParams(params);
  const field = params.get("suggestField");
  switch (field) {
    case "model":
      nextParams.delete("model");
      break;
    case "requestModel":
    case "responseModel":
      nextParams.delete("models");
      nextParams.delete("modelTarget");
      break;
    case "endpoint":
      nextParams.delete("endpoint");
      break;
    case "failureKind":
      nextParams.delete("failureKind");
      break;
    case "promptCacheKey":
      nextParams.delete("promptCacheKey");
      break;
    case "requesterIp":
      nextParams.delete("requesterIp");
      break;
    case "proxyDisplayName":
      nextParams.delete("proxyDisplayName");
      break;
    case "upstreamAccount":
      nextParams.delete("upstreamAccountId");
      break;
    case "serviceTier":
      nextParams.delete("serviceTier");
      break;
    case "reasoningEffort":
      nextParams.delete("reasoningEffort");
      nextParams.delete("reasoningEfforts");
      break;
    default:
      break;
  }
  nextParams.delete("suggestQuery");
  nextParams.delete("suggestField");
  return filterRecords(records, nextParams, attemptsByInvokeId);
}

function jsonResponse(payload: unknown) {
  return new Response(JSON.stringify(payload), {
    status: 200,
    headers: {
      "Content-Type": "application/json",
    },
  });
}

interface StorybookRecordsPageMockProps {
  children: ReactNode;
  newRecordsCount?: number;
  records?: ApiInvocation[];
  refreshDelayMs?: number;
}

function StorybookRecordsPageMock({
  children,
  newRecordsCount = 17,
  records = STORYBOOK_RECENT_INVOCATION_RECORDS,
  refreshDelayMs = 0,
}: StorybookRecordsPageMockProps) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null);
  const originalSetIntervalRef = useRef<typeof window.setInterval | null>(null);
  const invocationSearchCountRef = useRef(0);
  const poolAttemptsByInvokeId = useMemo(
    () => createStoryPoolAttemptsByInvokeId(records),
    [records],
  );
  const detailsById = useMemo(() => createStoryInvocationRecordDetailsById(records), [records]);
  const responseBodiesById = useMemo(
    () => createStoryInvocationResponseBodiesById(records),
    [records],
  );
  const recordsRef = useRef(records);
  const newRecordsCountRef = useRef(newRecordsCount);
  const refreshDelayMsRef = useRef(refreshDelayMs);
  const poolAttemptsByInvokeIdRef = useRef(poolAttemptsByInvokeId);
  const detailsByIdRef = useRef(detailsById);
  const responseBodiesByIdRef = useRef(responseBodiesById);

  recordsRef.current = records;
  newRecordsCountRef.current = newRecordsCount;
  refreshDelayMsRef.current = refreshDelayMs;
  poolAttemptsByInvokeIdRef.current = poolAttemptsByInvokeId;
  detailsByIdRef.current = detailsById;
  responseBodiesByIdRef.current = responseBodiesById;

  const maybeDelayRefresh = async () => {
    if (refreshDelayMsRef.current <= 0) return;
    await new Promise<void>((resolve) => {
      window.setTimeout(resolve, refreshDelayMsRef.current);
    });
  };

  if (typeof window !== "undefined" && !originalFetchRef.current) {
    originalFetchRef.current = window.fetch.bind(window);
    originalSetIntervalRef.current = window.setInterval.bind(window);

    window.setInterval = ((handler: TimerHandler, timeout?: number, ...args: unknown[]) => {
      if (timeout === 15_000) {
        return (originalSetIntervalRef.current as typeof window.setInterval)(
          handler,
          FAST_POLL_MS,
          ...args,
        );
      }
      return (originalSetIntervalRef.current as typeof window.setInterval)(
        handler,
        timeout,
        ...args,
      );
    }) as typeof window.setInterval;

    const mockedFetch: typeof window.fetch = async (input, init) => {
      const requestUrl =
        typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
      const url = new URL(requestUrl, window.location.origin);
      const path = url.pathname;
      const params = url.searchParams;
      const sortBy = (params.get("sortBy") as InvocationSortBy | null) ?? "occurredAt";
      const sortOrder = (params.get("sortOrder") as InvocationSortOrder | null) ?? "desc";
      const filtered = filterRecords(recordsRef.current, params, poolAttemptsByInvokeIdRef.current);
      const sorted = sortRecords(filtered, sortBy, sortOrder);

      if (path === "/api/invocations") {
        invocationSearchCountRef.current += 1;
        if (invocationSearchCountRef.current > 1) {
          await maybeDelayRefresh();
        }
        const { page, pageSize, paged } = paginateRecords(sorted, {
          page: params.get("page") ? Number(params.get("page")) : 1,
          pageSize: params.get("pageSize") ? Number(params.get("pageSize")) : 20,
        });
        return jsonResponse(
          createStoryInvocationRecordsResponse({
            snapshotId: Number(params.get("snapshotId") ?? SNAPSHOT_ID),
            total: sorted.length,
            page,
            pageSize,
            records: paged,
          }),
        );
      }

      if (path === "/api/invocations/summary") {
        if (invocationSearchCountRef.current > 1) {
          await maybeDelayRefresh();
        }
        const summary = summarizeInvocationRecords(sorted);
        return jsonResponse(
          createStoryInvocationRecordsSummary({
            snapshotId: Number(params.get("snapshotId") ?? SNAPSHOT_ID),
            ...summary.stats,
            token: summary.token,
            network: summary.network,
            exception: summary.exception,
          }),
        );
      }

      if (path === "/api/invocations/suggestions") {
        return jsonResponse(
          buildSuggestions(
            filterSuggestionSourceRecords(
              recordsRef.current,
              params,
              poolAttemptsByInvokeIdRef.current,
            ),
            params,
          ),
        );
      }

      if (path === "/api/invocations/new-count") {
        return jsonResponse({
          snapshotId: Number(params.get("snapshotId") ?? SNAPSHOT_ID),
          newRecordsCount: newRecordsCountRef.current,
        });
      }

      const poolAttemptsMatch = path.match(/^\/api\/invocations\/([^/]+)\/pool-attempts$/);
      if (poolAttemptsMatch) {
        const invokeId = decodeURIComponent(poolAttemptsMatch[1] ?? "");
        return jsonResponse(poolAttemptsByInvokeIdRef.current[invokeId] ?? []);
      }

      const detailMatch = path.match(/^\/api\/invocations\/(\d+)\/detail$/);
      if (detailMatch) {
        const recordId = Number(detailMatch[1] ?? "0");
        return jsonResponse(
          detailsByIdRef.current[recordId] ?? {
            id: recordId,
            abnormalResponseBody: null,
          },
        );
      }

      const responseBodyMatch = path.match(/^\/api\/invocations\/(\d+)\/response-body$/);
      if (responseBodyMatch) {
        const recordId = Number(responseBodyMatch[1] ?? "0");
        return jsonResponse(
          responseBodiesByIdRef.current[recordId] ?? {
            available: false,
            unavailableReason: "missing_body",
          },
        );
      }

      return (originalFetchRef.current as typeof window.fetch)(input, init);
    };

    window.fetch = mockedFetch;
  }

  useEffect(() => {
    return () => {
      if (typeof window !== "undefined") {
        if (originalFetchRef.current) {
          window.fetch = originalFetchRef.current;
          originalFetchRef.current = null;
        }
        if (originalSetIntervalRef.current) {
          window.setInterval = originalSetIntervalRef.current;
          originalSetIntervalRef.current = null;
        }
      }
    };
  }, []);

  return <>{children}</>;
}

type RecordsStoryParameters = {
  newRecordsCount?: number;
  records?: ApiInvocation[];
  refreshDelayMs?: number;
};

const meta = {
  title: "Records/RecordsPage",
  component: RecordsPage,
  parameters: {
    layout: "fullscreen",
    viewport: { defaultViewport: "desktop1660" },
  },
  decorators: [
    (Story, context) => {
      const { newRecordsCount, records, refreshDelayMs } =
        context.parameters as RecordsStoryParameters;
      return (
        <MemoryRouter>
          <I18nProvider>
            <StorybookRecordsPageMock
              newRecordsCount={newRecordsCount}
              records={records}
              refreshDelayMs={refreshDelayMs}
            >
              <StorySurface>
                <Story />
              </StorySurface>
            </StorybookRecordsPageMock>
          </I18nProvider>
        </MemoryRouter>
      );
    },
  ],
} satisfies Meta<typeof RecordsPage>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Default: Story = {
  parameters: {
    newRecordsCount: 17,
  },
  render: () => <RecordsPage />,
};

export const ProxyFilterRemoved: Story = {
  parameters: {
    newRecordsCount: 0,
  },
  render: () => <RecordsPage />,
};

export const NoNewData: Story = {
  parameters: {
    newRecordsCount: 0,
  },
  render: () => <RecordsPage />,
};

export const EmptyResults: Story = {
  parameters: {
    newRecordsCount: 0,
    records: [],
  },
  render: () => <RecordsPage />,
};

export const RefreshingNewData: Story = {
  parameters: {
    newRecordsCount: 17,
    refreshDelayMs: 1600,
  },
  render: () => <RecordsPage />,
  play: async ({ canvasElement }) => {
    const doc = canvasElement.ownerDocument;
    window.setTimeout(() => {
      const button = doc.querySelector('[data-testid="records-new-data-button"]');
      if (button instanceof HTMLButtonElement) {
        button.click();
      }
    }, 300);
  },
};

export const AutocompleteSuppressedFilters: Story = {
  parameters: {
    newRecordsCount: 0,
  },
  render: () => <RecordsPage />,
  play: async ({ canvasElement }) => {
    const doc = canvasElement.ownerDocument;
    await userEvent.click(within(canvasElement).getByTestId("records-open-filters"));
    await userEvent.click(within(doc.body).getByTestId("records-filter-model-selection-trigger"));
    await userEvent.click(
      within(doc.body).getByRole("combobox", {
        name: /模型|model/i,
      }),
    );

    const modelInput = doc.querySelector("#records-filter-model-input");
    const keywordInput = doc.querySelector('input[name="keyword"]');
    const statusSelect = within(canvasElement).getByRole("button", {
      name: /状态|status/i,
    });

    if (!(modelInput instanceof HTMLInputElement)) {
      throw new Error("missing records model combobox input");
    }
    if (!(keywordInput instanceof HTMLInputElement)) {
      throw new Error("missing records keyword input");
    }
    if (!(statusSelect instanceof HTMLButtonElement)) {
      throw new Error("missing records status trigger");
    }

    await expect(modelInput.getAttribute("autocomplete")).toBe("off");
    await expect(modelInput.getAttribute("autocorrect")).toBe("off");
    await expect(modelInput.getAttribute("autocapitalize")).toBe("none");
    await expect(modelInput.getAttribute("spellcheck")).toBe("false");

    await expect(keywordInput.getAttribute("autocomplete")).toBe("off");
    await expect(keywordInput.getAttribute("autocorrect")).toBe("off");
    await expect(keywordInput.getAttribute("autocapitalize")).toBe("none");
    await expect(keywordInput.getAttribute("spellcheck")).toBe("false");

    await expect(doc.querySelector('select[name="status"]')).toBeNull();

    await userEvent.click(modelInput);

    const listbox = doc.body.querySelector('[role="listbox"]');
    if (!(listbox instanceof HTMLElement)) {
      throw new Error("missing records combobox listbox");
    }

    await expect(listbox).toBeVisible();
    await expect(listbox.textContent ?? "").toContain("gpt-5.4");
  },
};

export const MobileFiltersDrawer: Story = {
  parameters: {
    newRecordsCount: 0,
    viewport: { defaultViewport: "mobile390" },
  },
  render: () => <RecordsPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const doc = canvasElement.ownerDocument;

    await userEvent.click(canvas.getByTestId("records-open-filters"));

    await waitFor(() => {
      expect(doc.querySelector('[data-testid="records-filters-drawer"]')).not.toBeNull();
      expect(doc.querySelector('[role="dialog"]')).not.toBeNull();
    });
  },
};

export const RangeDiagnosticsDrawer: Story = {
  parameters: {
    newRecordsCount: 0,
  },
  render: () => <RecordsPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const doc = canvasElement.ownerDocument;

    await userEvent.click(canvas.getByTestId("records-open-filters"));

    const timeRangeField = doc.querySelector('[data-testid="records-filter-time-range"]');
    if (!(timeRangeField instanceof HTMLElement)) {
      throw new Error("missing time range field");
    }
    const timeRangeTrigger = timeRangeField.querySelector("button");
    if (!(timeRangeTrigger instanceof HTMLButtonElement)) {
      throw new Error("missing time range trigger");
    }

    await userEvent.click(timeRangeTrigger);

    await waitFor(() => {
      expect(doc.querySelector('input[name="customFrom"]')).not.toBeNull();
      expect(doc.querySelector('[data-testid="records-filter-total-tokens-range"]')).not.toBeNull();
      expect(doc.querySelector('[data-testid="records-filter-total-ms-range"]')).not.toBeNull();
    });
  },
};

export const ModelFilterDrawer: Story = {
  parameters: {
    newRecordsCount: 0,
  },
  render: () => <RecordsPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const doc = canvasElement.ownerDocument;

    await userEvent.click(canvas.getByTestId("records-open-filters"));
    await userEvent.click(within(doc.body).getByTestId("records-filter-model-selection-trigger"));

    const responseButton = within(doc.body).getByRole("button", {
      name: /响应|response/i,
    });
    const reroutedButton = within(doc.body).getByRole("button", {
      name: /重路由:\s*(已重路由|rerouted)/i,
    });

    await userEvent.click(responseButton);
    await userEvent.click(reroutedButton);

    const modelTrigger = within(doc.body).getByRole("combobox", {
      name: /模型|model/i,
    });
    await userEvent.click(modelTrigger);

    const modelInput = doc.body.querySelector(
      '[cmdk-input][aria-label="模型"], [cmdk-input][aria-label="Model"]',
    );

    if (!(modelInput instanceof HTMLInputElement)) {
      throw new Error("missing records model input");
    }
    await userEvent.type(modelInput, "gpt-5.4");

    await waitFor(() => {
      const option = Array.from(doc.body.querySelectorAll("[cmdk-item]")).find((candidate) =>
        (candidate.textContent || "").includes("gpt-5.4"),
      );
      expect(option).toBeTruthy();
    });

    const modelOption = Array.from(doc.body.querySelectorAll("[cmdk-item]")).find((candidate) =>
      (candidate.textContent || "").includes("gpt-5.4"),
    );
    if (!(modelOption instanceof HTMLElement)) {
      throw new Error("missing model option");
    }
    await userEvent.click(modelOption);

    const reasoningTrigger = within(doc.body).getByRole("combobox", {
      name: /推理强度|reasoning effort/i,
    });
    await userEvent.click(reasoningTrigger);

    const reasoningInput = doc.body.querySelector(
      '[cmdk-input][aria-label="推理强度"], [cmdk-input][aria-label="Reasoning effort"]',
    );
    if (!(reasoningInput instanceof HTMLInputElement)) {
      throw new Error("missing records reasoning effort input");
    }

    await userEvent.type(reasoningInput, "high");

    await waitFor(() => {
      const option = Array.from(doc.body.querySelectorAll("[cmdk-item]")).find((candidate) =>
        (candidate.textContent || "").includes("high"),
      );
      expect(option).toBeTruthy();
    });

    const reasoningOption = Array.from(doc.body.querySelectorAll("[cmdk-item]")).find((candidate) =>
      (candidate.textContent || "").includes("high"),
    );
    if (!(reasoningOption instanceof HTMLElement)) {
      throw new Error("missing reasoning effort option");
    }
    await userEvent.click(reasoningOption);

    await waitFor(() => {
      expect(doc.body.textContent ?? "").toContain("gpt-5.4");
      expect(doc.body.textContent ?? "").toContain("high");
    });
    await expect(reroutedButton).toHaveAttribute("data-active", "true");
  },
};

export const PoolDetailsExpanded: Story = {
  parameters: {
    newRecordsCount: 0,
    records: STORYBOOK_POOL_DETAILS_RECORDS,
  },
  render: () => <RecordsPage />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const doc = canvasElement.ownerDocument;

    await expect(canvas.getByRole("heading", { name: /记录|records/i })).toBeInTheDocument();

    await userEvent.click(canvas.getByRole("tab", { name: /网络|network/i }));

    let detailToggle: HTMLButtonElement | null = null;
    await waitFor(() => {
      detailToggle =
        Array.from(doc.querySelectorAll('button[aria-expanded="false"]')).find(
          (element): element is HTMLButtonElement =>
            element instanceof HTMLButtonElement && element.offsetParent !== null,
        ) ?? null;
      expect(detailToggle).not.toBeNull();
    });

    await userEvent.click(detailToggle!);

    await waitFor(() => {
      expect(doc.querySelector('[data-testid="records-detail-summary-strip"]')).not.toBeNull();
      expect(doc.querySelector('[data-testid="pool-attempts-list"]')).not.toBeNull();
    });

    await expect(doc.body.textContent ?? "").toContain("Pool Alpha 17");
  },
};
