/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  FetchUpstreamAccountsQuery,
  ForwardProxyBindingNode,
  RateWindowActualUsage,
  UpstreamAccountDetail,
  UpstreamAccountGroupSummary,
  UpstreamAccountListResponse,
  UpstreamAccountSummary,
  UpstreamAccountWindowUsageResponse,
} from "../lib/api";
import {
  UPSTREAM_ACCOUNTS_OPEN_RESYNC_COOLDOWN_MS,
  UPSTREAM_ACCOUNTS_SSE_REFRESH_THROTTLE_MS,
  useUpstreamAccounts,
} from "./useUpstreamAccounts";

const apiMocks = vi.hoisted(() => ({
  fetchUpstreamAccounts: vi.fn<
    (query?: FetchUpstreamAccountsQuery) => Promise<UpstreamAccountListResponse>
  >(),
  fetchUpstreamAccountDetail: vi.fn<
    (accountId: number, signal?: AbortSignal) => Promise<UpstreamAccountDetail>
  >(),
  fetchUpstreamAccountWindowUsage: vi.fn<
    (accountIds: number[]) => Promise<UpstreamAccountWindowUsageResponse>
  >(),
  updateUpstreamAccountGroup: vi.fn<
    (
      groupName: string,
      payload: import("../lib/api").UpdateUpstreamAccountGroupPayload,
    ) => Promise<UpstreamAccountGroupSummary>
  >(),
  syncUpstreamAccount: vi.fn<(accountId: number) => Promise<UpstreamAccountDetail>>(),
  reloginUpstreamAccount: vi.fn<(accountId: number) => Promise<{ loginId: string }>>(),
  deleteUpstreamAccount: vi.fn<(accountId: number) => Promise<void>>(),
  deleteUpstreamAccountGroup: vi.fn<(groupName: string) => Promise<void>>(),
}));

const sseMocks = vi.hoisted(() => ({
  recordListeners: [] as Array<(payload: { type: string; records?: unknown[] }) => void>,
  openListeners: [] as Array<() => void>,
}));

vi.mock("../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../lib/api")>("../lib/api");
  return {
    ...actual,
    fetchUpstreamAccounts: apiMocks.fetchUpstreamAccounts,
    fetchUpstreamAccountDetail: apiMocks.fetchUpstreamAccountDetail,
    fetchUpstreamAccountWindowUsage: apiMocks.fetchUpstreamAccountWindowUsage,
    updateUpstreamAccountGroup: apiMocks.updateUpstreamAccountGroup,
    syncUpstreamAccount: apiMocks.syncUpstreamAccount,
    reloginUpstreamAccount: apiMocks.reloginUpstreamAccount,
    deleteUpstreamAccount: apiMocks.deleteUpstreamAccount,
    deleteUpstreamAccountGroup: apiMocks.deleteUpstreamAccountGroup,
  };
});

vi.mock("../lib/sse", () => ({
  subscribeToSse: (
    listener: (payload: { type: string; records?: unknown[] }) => void,
  ) => {
    sseMocks.recordListeners.push(listener);
    return () => {
      const index = sseMocks.recordListeners.indexOf(listener);
      if (index >= 0) {
        sseMocks.recordListeners.splice(index, 1);
      }
    };
  },
  subscribeToSseOpen: (listener: () => void) => {
    sseMocks.openListeners.push(listener);
    return () => {
      const index = sseMocks.openListeners.indexOf(listener);
      if (index >= 0) {
        sseMocks.openListeners.splice(index, 1);
      }
    };
  },
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});

beforeEach(() => {
  vi.resetAllMocks();
  sseMocks.recordListeners.length = 0;
  sseMocks.openListeners.length = 0;
  apiMocks.fetchUpstreamAccounts.mockResolvedValue(createListResponse());
  apiMocks.updateUpstreamAccountGroup.mockResolvedValue(createGroupSummary("prod"));
  apiMocks.deleteUpstreamAccountGroup.mockResolvedValue();
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

function rerender(ui: React.ReactNode) {
  act(() => {
    root?.render(ui);
  });
}

async function flushAsync() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function text(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`);
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing element: ${testId}`);
  }
  return element.textContent ?? "";
}

function click(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`);
  if (!(element instanceof HTMLButtonElement)) {
    throw new Error(`Missing button: ${testId}`);
  }
  act(() => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function emitRecordsEvent() {
  for (const listener of [...sseMocks.recordListeners]) {
    listener({ type: "records", records: [] });
  }
}

function emitOpenEvent() {
  for (const listener of [...sseMocks.openListeners]) {
    listener();
  }
}

function createRateWindowActualUsage(
  requestCount: number,
  totalTokens: number,
  totalCost: number,
): RateWindowActualUsage {
  const cacheInputTokens = Math.round(totalTokens * 0.1);
  const inputTokens = Math.round(totalTokens * 0.55);
  const outputTokens = totalTokens - inputTokens - cacheInputTokens;
  return {
    requestCount,
    totalTokens,
    totalCost,
    inputTokens,
    outputTokens,
    cacheInputTokens,
  };
}

function createSummary(
  id: number,
  displayName: string,
): UpstreamAccountSummary {
  return {
    id,
    kind: "oauth_codex",
    provider: "codex",
    displayName,
    groupName: "prod",
    isMother: false,
    status: "active",
    enabled: true,
    tags: [],
    effectiveRoutingRule: {
      blockNewConversations: false,
      allowCutOut: false,
      allowCutIn: false,
      sourceTagIds: [],
      sourceTagNames: [],
    },
  };
}

function createDetail(id: number, displayName: string): UpstreamAccountDetail {
  return {
    ...createSummary(id, displayName),
    email: `${displayName.toLowerCase().replace(/\s+/g, ".")}@example.com`,
    history: [],
  };
}

function createGroupSummary(
  groupName: string,
  overrides: Partial<UpstreamAccountGroupSummary> = {},
): UpstreamAccountGroupSummary {
  return {
    groupName,
    accountCount: 2,
    note: null,
    boundProxyKeys: [],
    concurrencyLimit: null,
    nodeShuntEnabled: false,
    upstream429RetryEnabled: false,
    upstream429MaxRetries: 0,
    ...overrides,
  };
}

function createWindowedSummary(
  id: number,
  displayName: string,
): UpstreamAccountSummary {
  return {
    ...createSummary(id, displayName),
    primaryWindow: {
      usedPercent: 42,
      usedText: "42% used",
      limitText: "5h rolling window",
      resetsAt: "2026-03-29T14:27:00.000Z",
      windowDurationMins: 300,
      actualUsage: null,
    },
    secondaryWindow: {
      usedPercent: 18,
      usedText: "18% used",
      limitText: "7d rolling window",
      resetsAt: "2026-04-05T14:27:00.000Z",
      windowDurationMins: 10080,
      actualUsage: null,
    },
    localLimits: {
      primaryLimit: null,
      secondaryLimit: null,
      limitUnit: "requests",
    },
  };
}

function createWindowUsageResponse(accountIds: number[]): UpstreamAccountWindowUsageResponse {
  return {
    items: accountIds.map((accountId, index) => ({
      accountId,
      primaryActualUsage: createRateWindowActualUsage(
        10 + index,
        20_000 + accountId * 100,
        Number((0.4 + index * 0.05).toFixed(4)),
      ),
      secondaryActualUsage: createRateWindowActualUsage(
        30 + index,
        80_000 + accountId * 200,
        Number((1.2 + index * 0.08).toFixed(4)),
      ),
    })),
  };
}

function createForwardProxyNode(
  key: string,
  displayName = "JP Edge 01",
): ForwardProxyBindingNode {
  return {
    key,
    source: "manual",
    displayName,
    protocolLabel: "HTTP",
    penalized: false,
    selectable: true,
    last24h: [],
  };
}

function createListResponse(
  overrides: Partial<UpstreamAccountListResponse> = {},
): UpstreamAccountListResponse {
  return {
    writesEnabled: true,
    items: [createSummary(1, "Alpha"), createSummary(2, "Beta")],
    groups: [],
    forwardProxyNodes: [],
    hasUngroupedAccounts: false,
    routing: { writesEnabled: true, apiKeyConfigured: false, maskedApiKey: null },
    ...overrides,
  };
}

function Probe({ query }: { query?: FetchUpstreamAccountsQuery | null }) {
  const {
    items,
    selectedId,
    selectedSummary,
    detail,
    isDetailLoading,
    listError,
    listState,
    forwardProxyCatalogState,
    isWindowUsagePending,
    detailError,
    error,
    selectAccount,
    runSync,
    refresh,
    hydrateWindowUsage,
    beginRelogin,
    removeAccount,
    saveGroupNote,
  } =
    useUpstreamAccounts(query);

  return (
    <div>
      <div data-testid="selected-id">{selectedId ?? ""}</div>
      <div data-testid="selected-name">{selectedSummary?.displayName ?? ""}</div>
      <div data-testid="detail-id">{detail?.id ?? ""}</div>
      <div data-testid="detail-name">{detail?.displayName ?? ""}</div>
      <div data-testid="detail-loading">{isDetailLoading ? "true" : "false"}</div>
      <div data-testid="list-error">{listError ?? ""}</div>
      <div data-testid="list-freshness">{listState.freshness}</div>
      <div data-testid="list-loading-state">{listState.loadingState}</div>
      <div data-testid="list-status">{listState.status}</div>
      <div data-testid="list-has-current-query-data">
        {listState.hasCurrentQueryData ? "true" : "false"}
      </div>
      <div data-testid="proxy-catalog-kind">{forwardProxyCatalogState.kind}</div>
      <div data-testid="proxy-catalog-freshness">
        {forwardProxyCatalogState.freshness}
      </div>
      <div data-testid="window-usage-pending">
        {isWindowUsagePending ? "true" : "false"}
      </div>
      <div data-testid="first-item-id">{items[0]?.id ?? ""}</div>
      <div data-testid="first-item-primary-requests">
        {items[0]?.primaryWindow?.actualUsage?.requestCount ?? ""}
      </div>
      <div data-testid="first-item-secondary-requests">
        {items[0]?.secondaryWindow?.actualUsage?.requestCount ?? ""}
      </div>
      <div data-testid="detail-error">{detailError ?? ""}</div>
      <div data-testid="error">{error ?? ""}</div>
      <button data-testid="select-beta" onClick={() => selectAccount(2)}>
        select beta
      </button>
      <button data-testid="select-alpha" onClick={() => selectAccount(1)}>
        select alpha
      </button>
      <button data-testid="select-gamma" onClick={() => selectAccount(3)}>
        select gamma
      </button>
      <button data-testid="sync-alpha" onClick={() => void runSync(1)}>
        sync alpha
      </button>
      <button data-testid="refresh" onClick={() => void refresh()}>
        refresh
      </button>
      <button
        data-testid="hydrate-visible"
        onClick={() => void hydrateWindowUsage(items.map((item) => item.id))}
      >
        hydrate visible
      </button>
      <button data-testid="relogin-alpha" onClick={() => void beginRelogin(1)}>
        relogin alpha
      </button>
      <button data-testid="remove-alpha" onClick={() => void removeAccount(1)}>
        remove alpha
      </button>
      <button
        data-testid="save-prod-group"
        onClick={() => void saveGroupNote("prod", { routingRule: { priorityTier: "fallback" } })}
      >
        save prod group
      </button>
    </div>
  );
}

describe("useUpstreamAccounts", () => {
  it("defers the roster request until the query is available", async () => {
    render(<Probe query={null} />);
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccounts).not.toHaveBeenCalled();
  });

  it("passes server-side roster filters through to the list endpoint", async () => {
    render(
      <Probe
        query={{
          groupSearch: "prod",
          workStatus: ["working", "rate_limited"],
          healthStatus: ["normal"],
          tagIds: [1, 2],
        }}
      />,
    );
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledWith({
      groupSearch: "prod",
      workStatus: ["working", "rate_limited"],
      healthStatus: ["normal"],
      tagIds: [1, 2],
    });
  });

  it("treats grouped includeAll queries as a distinct roster key", async () => {
    apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));

    render(<Probe query={{ page: 1, pageSize: 20 }} />);
    await flushAsync();

    rerender(<Probe query={{ includeAll: true }} />);
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccounts).toHaveBeenNthCalledWith(1, {
      page: 1,
      pageSize: 20,
    });
    expect(apiMocks.fetchUpstreamAccounts).toHaveBeenNthCalledWith(2, {
      includeAll: true,
    });
  });

  it("auto-hydrates window usage only for the selected account", async () => {
    const hydration = deferred<UpstreamAccountWindowUsageResponse>();
    apiMocks.fetchUpstreamAccounts.mockResolvedValueOnce(
      createListResponse({
        items: [createWindowedSummary(1, "Alpha"), createWindowedSummary(2, "Beta")],
      }),
    );
    apiMocks.fetchUpstreamAccountWindowUsage.mockImplementationOnce(
      async () => hydration.promise,
    );

    render(<Probe query={{ page: 1, pageSize: 20 }} />);
    await flushAsync();

    expect(text("window-usage-pending")).toBe("true");
    expect(text("first-item-primary-requests")).toBe("");
    expect(apiMocks.fetchUpstreamAccountWindowUsage).toHaveBeenCalledWith([1]);

    hydration.resolve(createWindowUsageResponse([1]));
    await flushAsync();

    expect(text("window-usage-pending")).toBe("false");
    expect(text("first-item-primary-requests")).toBe("10");
    expect(text("first-item-secondary-requests")).toBe("30");
  });

  it("hydrates only the selected account for includeAll roster queries", async () => {
    apiMocks.fetchUpstreamAccounts.mockResolvedValueOnce(
      createListResponse({
        items: [createWindowedSummary(1, "Alpha"), createWindowedSummary(2, "Beta")],
      }),
    );
    apiMocks.fetchUpstreamAccountWindowUsage.mockResolvedValueOnce(
      createWindowUsageResponse([1, 2]),
    );

    render(<Probe query={{ includeAll: true }} />);
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccountWindowUsage).toHaveBeenCalledWith([1]);
    expect(text("window-usage-pending")).toBe("false");
    expect(text("first-item-primary-requests")).toBe("10");

    click("hydrate-visible");
    await flushAsync();
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccountWindowUsage).toHaveBeenCalledWith([2]);
    expect(text("first-item-primary-requests")).toBe("10");
  });

  it("drops stale selected-account window-usage responses after the roster query changes", async () => {
    const firstHydration = deferred<UpstreamAccountWindowUsageResponse>();
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(
        createListResponse({
          items: [createWindowedSummary(1, "Alpha"), createWindowedSummary(2, "Beta")],
        }),
      )
      .mockResolvedValueOnce(
        createListResponse({
          items: [createWindowedSummary(3, "Gamma"), createWindowedSummary(4, "Delta")],
          total: 4,
          page: 2,
          pageSize: 20,
        }),
      );
    apiMocks.fetchUpstreamAccountWindowUsage.mockImplementationOnce(
      async () => firstHydration.promise,
    );

    render(<Probe query={{ page: 1, pageSize: 20 }} />);
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccountWindowUsage).toHaveBeenCalledWith([1]);
    expect(text("window-usage-pending")).toBe("true");

    rerender(<Probe query={{ includeAll: true }} />);
    await flushAsync();

    expect(text("first-item-id")).toBe("3");
    expect(text("window-usage-pending")).toBe("false");
    expect(text("first-item-primary-requests")).toBe("");

    firstHydration.resolve(createWindowUsageResponse([1]));
    await flushAsync();

    expect(text("first-item-id")).toBe("3");
    expect(text("first-item-primary-requests")).toBe("");
    expect(text("first-item-secondary-requests")).toBe("");
  });

  it("keeps selected-account window-usage pending while a manual rehydrate supersedes an older request", async () => {
    const firstHydration = deferred<UpstreamAccountWindowUsageResponse>();
    const secondHydration = deferred<UpstreamAccountWindowUsageResponse>();
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(
        createListResponse({
          items: [createWindowedSummary(1, "Alpha"), createWindowedSummary(2, "Beta")],
        }),
      )
      .mockResolvedValueOnce(
        createListResponse({
          items: [createWindowedSummary(1, "Alpha"), createWindowedSummary(2, "Beta")],
        }),
      );
    apiMocks.fetchUpstreamAccountWindowUsage
      .mockImplementationOnce(async () => firstHydration.promise)
      .mockImplementationOnce(async () => secondHydration.promise)
      .mockResolvedValueOnce(createWindowUsageResponse([2]));

    render(<Probe query={{ page: 1, pageSize: 20 }} />);
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccountWindowUsage).toHaveBeenNthCalledWith(1, [1]);
    expect(text("window-usage-pending")).toBe("true");

    click("hydrate-visible");
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccountWindowUsage).toHaveBeenNthCalledWith(2, [2]);
    expect(text("window-usage-pending")).toBe("true");

    firstHydration.resolve(createWindowUsageResponse([1]));
    await flushAsync();

    expect(text("window-usage-pending")).toBe("true");
    expect(text("first-item-primary-requests")).toBe("10");

    secondHydration.resolve(createWindowUsageResponse([2]));
    await flushAsync();

    expect(text("window-usage-pending")).toBe("false");
    expect(text("first-item-primary-requests")).toBe("10");
    expect(text("first-item-secondary-requests")).toBe("30");
  });

  it("marks a query switch as stale until the new roster lands", async () => {
    const nextPage = deferred<UpstreamAccountListResponse>();
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockImplementationOnce(async () => nextPage.promise);
    apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));

    render(<Probe query={{ page: 1, pageSize: 20 }} />);
    await flushAsync();

    expect(text("list-freshness")).toBe("fresh");
    expect(text("list-loading-state")).toBe("idle");
    expect(text("list-status")).toBe("ready");
    expect(text("list-has-current-query-data")).toBe("true");

    rerender(<Probe query={{ page: 2, pageSize: 20 }} />);
    await flushAsync();

    expect(text("selected-name")).toBe("Alpha");
    expect(text("list-freshness")).toBe("stale");
    expect(text("list-loading-state")).toBe("switching");
    expect(text("list-status")).toBe("loading");
    expect(text("list-has-current-query-data")).toBe("false");

    nextPage.resolve({
      ...createListResponse(),
      items: [createSummary(3, "Gamma"), createSummary(4, "Delta")],
      total: 4,
      page: 2,
      pageSize: 20,
    });
    await flushAsync();

    expect(text("selected-id")).toBe("3");
    expect(text("selected-name")).toBe("Gamma");
    expect(text("list-freshness")).toBe("fresh");
    expect(text("list-loading-state")).toBe("idle");
    expect(text("list-status")).toBe("ready");
    expect(text("list-has-current-query-data")).toBe("true");
  });

  it("does not refetch the roster when rerenders keep the same query key", async () => {
    apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));

    render(<Probe query={{ page: 1, pageSize: 20 }} />);
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(1);

    rerender(<Probe query={{ page: 1, pageSize: 20 }} />);
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(1);
  });

  it("keeps the forward proxy catalog in loading state until the first roster payload lands", async () => {
    const listRequest = deferred<UpstreamAccountListResponse>();
    apiMocks.fetchUpstreamAccounts.mockReturnValueOnce(listRequest.promise);

    render(<Probe />);

    expect(text("proxy-catalog-kind")).toBe("loading");
    expect(text("proxy-catalog-freshness")).toBe("missing");

    listRequest.resolve(
      createListResponse({
        forwardProxyNodes: [createForwardProxyNode("jp-edge-01")],
      }),
    );
    await flushAsync();

    expect(text("proxy-catalog-kind")).toBe("ready-with-data");
    expect(text("proxy-catalog-freshness")).toBe("fresh");
  });

  it("reports an empty-but-loaded forward proxy catalog distinctly from loading", async () => {
    apiMocks.fetchUpstreamAccounts.mockResolvedValueOnce(
      createListResponse({ forwardProxyNodes: [] }),
    );

    render(<Probe />);
    await flushAsync();

    expect(text("proxy-catalog-kind")).toBe("ready-empty");
    expect(text("proxy-catalog-freshness")).toBe("fresh");
  });

  it("treats a pending refresh of an empty proxy catalog as loading until the refreshed roster lands", async () => {
    const refreshedList = deferred<UpstreamAccountListResponse>();
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse({ forwardProxyNodes: [] }))
      .mockImplementationOnce(async () => refreshedList.promise);

    render(<Probe />);
    await flushAsync();

    expect(text("proxy-catalog-kind")).toBe("ready-empty");
    expect(text("proxy-catalog-freshness")).toBe("fresh");

    act(() => {
      (
        host?.querySelector('[data-testid="refresh"]') as HTMLButtonElement | null
      )?.click();
    });
    await flushAsync();

    expect(text("proxy-catalog-kind")).toBe("loading");
    expect(text("proxy-catalog-freshness")).toBe("stale");

    refreshedList.resolve(
      createListResponse({
        forwardProxyNodes: [createForwardProxyNode("jp-edge-01")],
      }),
    );
    await flushAsync();

    expect(text("proxy-catalog-kind")).toBe("ready-with-data");
    expect(text("proxy-catalog-freshness")).toBe("fresh");
  });

  it("keeps an empty proxy catalog stale after a refresh failure", async () => {
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse({ forwardProxyNodes: [] }))
      .mockRejectedValueOnce(new Error("refresh failed"));

    render(<Probe />);
    await flushAsync();

    expect(text("proxy-catalog-kind")).toBe("ready-empty");
    expect(text("proxy-catalog-freshness")).toBe("fresh");

    act(() => {
      (
        host?.querySelector('[data-testid="refresh"]') as HTMLButtonElement | null
      )?.click();
    });
    await flushAsync();

    expect(text("list-error")).toBe("refresh failed");
    expect(text("proxy-catalog-kind")).toBe("ready-empty");
    expect(text("proxy-catalog-freshness")).toBe("stale");
  });

  it("keeps a populated proxy catalog stale after a refresh failure", async () => {
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(
        createListResponse({
          forwardProxyNodes: [createForwardProxyNode("jp-edge-01")],
        }),
      )
      .mockRejectedValueOnce(new Error("refresh failed"));

    render(<Probe />);
    await flushAsync();

    expect(text("proxy-catalog-kind")).toBe("ready-with-data");
    expect(text("proxy-catalog-freshness")).toBe("fresh");

    act(() => {
      (
        host?.querySelector('[data-testid="refresh"]') as HTMLButtonElement | null
      )?.click();
    });
    await flushAsync();

    expect(text("list-error")).toBe("refresh failed");
    expect(text("proxy-catalog-kind")).toBe("ready-with-data");
    expect(text("proxy-catalog-freshness")).toBe("stale");
  });

  it("reports the current query as failed after a switched roster request rejects", async () => {
    const nextPage = deferred<UpstreamAccountListResponse>();
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockImplementationOnce(async () => nextPage.promise);
    apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));

    render(<Probe query={{ page: 1, pageSize: 20 }} />);
    await flushAsync();

    rerender(<Probe query={{ page: 2, pageSize: 20 }} />);
    await flushAsync();

    nextPage.reject(new Error("page two failed"));
    await flushAsync();

    expect(text("selected-name")).toBe("Alpha");
    expect(text("list-error")).toBe("page two failed");
    expect(text("list-freshness")).toBe("stale");
    expect(text("list-loading-state")).toBe("idle");
    expect(text("list-status")).toBe("error");
    expect(text("list-has-current-query-data")).toBe("false");
  });

  it("ignores stale detail responses after account switches", async () => {
    const first = deferred<UpstreamAccountDetail>();
    const second = deferred<UpstreamAccountDetail>();
    apiMocks.fetchUpstreamAccountDetail
      .mockImplementationOnce(async () => first.promise)
      .mockImplementationOnce(async () => second.promise);

    render(<Probe />);
    await flushAsync();

    expect(text("selected-id")).toBe("1");
    click("select-beta");
    await flushAsync();

    second.resolve(createDetail(2, "Beta"));
    await flushAsync();
    expect(text("detail-id")).toBe("2");
    expect(text("detail-name")).toBe("Beta");

    first.resolve(createDetail(1, "Alpha"));
    await flushAsync();
    expect(text("selected-id")).toBe("2");
    expect(text("detail-id")).toBe("2");
    expect(text("detail-name")).toBe("Beta");
  });

  it("ignores stale detail errors after account switches", async () => {
    const first = deferred<UpstreamAccountDetail>();
    const second = deferred<UpstreamAccountDetail>();
    apiMocks.fetchUpstreamAccountDetail
      .mockImplementationOnce(async () => first.promise)
      .mockImplementationOnce(async () => second.promise);

    render(<Probe />);
    await flushAsync();

    click("select-beta");
    await flushAsync();

    second.resolve(createDetail(2, "Beta"));
    await flushAsync();

    first.reject(new Error("Alpha failed"));
    await flushAsync();

    expect(text("selected-id")).toBe("2");
    expect(text("detail-id")).toBe("2");
    expect(text("detail-name")).toBe("Beta");
    expect(text("error")).toBe("");
  });

  it("invalidates the previous detail request in the same turn as a selection change", async () => {
    const first = deferred<UpstreamAccountDetail>();
    const second = deferred<UpstreamAccountDetail>();
    apiMocks.fetchUpstreamAccountDetail
      .mockImplementationOnce(async () => first.promise)
      .mockImplementationOnce(async () => second.promise);

    render(<Probe />);
    await flushAsync();

    click("select-beta");
    first.reject(new Error("Alpha failed"));
    await flushAsync();

    second.resolve(createDetail(2, "Beta"));
    await flushAsync();

    expect(text("selected-id")).toBe("2");
    expect(text("detail-id")).toBe("2");
    expect(text("detail-name")).toBe("Beta");
    expect(text("error")).toBe("");
  });

  it("does not reclaim selection when an older account sync finishes later", async () => {
    const sync = deferred<UpstreamAccountDetail>();
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockResolvedValueOnce(createDetail(2, "Beta"))
      .mockResolvedValue(createDetail(2, "Beta"));
    apiMocks.syncUpstreamAccount.mockImplementationOnce(async () => sync.promise);

    render(<Probe />);
    await flushAsync();

    expect(text("selected-id")).toBe("1");
    expect(text("detail-id")).toBe("1");

    click("sync-alpha");
    click("select-beta");
    await flushAsync();
    expect(text("selected-id")).toBe("2");

    sync.resolve(createDetail(1, "Alpha"));
    await flushAsync();

    expect(text("selected-id")).toBe("2");
    expect(text("selected-name")).toBe("Beta");
    expect(text("detail-id")).not.toBe("1");
    expect(text("detail-name")).not.toBe("Alpha");
  });

  it("reloads the currently selected account detail after another account sync finishes", async () => {
    const sync = deferred<UpstreamAccountDetail>();
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockResolvedValueOnce(createListResponse());
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockResolvedValueOnce(createDetail(2, "Beta Stale"))
      .mockResolvedValueOnce(createDetail(2, "Beta Fresh"));
    apiMocks.syncUpstreamAccount.mockImplementationOnce(async () => sync.promise);

    render(<Probe />);
    await flushAsync();

    click("sync-alpha");
    click("select-beta");
    await flushAsync();
    expect(text("selected-id")).toBe("2");
    expect(text("detail-name")).toBe("Beta Stale");

    sync.resolve(createDetail(1, "Alpha Synced"));
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenNthCalledWith(
      3,
      2,
      expect.any(AbortSignal),
    );
    expect(text("selected-id")).toBe("2");
  });

  it("reloads the currently selected account detail after group settings save", async () => {
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockResolvedValueOnce(createListResponse());
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockResolvedValue(createDetail(1, "Alpha Group Policy Fresh"));
    apiMocks.updateUpstreamAccountGroup.mockResolvedValueOnce(
      createGroupSummary("prod", {
        routingRule: {
          blockNewConversations: false,
          allowCutOut: false,
          allowCutIn: false,
          priorityTier: "fallback",
        },
      }),
    );

    render(<Probe />);
    await flushAsync();

    expect(text("detail-name")).toBe("Alpha");
    click("save-prod-group");
    await flushAsync();

    expect(apiMocks.updateUpstreamAccountGroup).toHaveBeenCalledWith("prod", {
      routingRule: { priorityTier: "fallback" },
    });
    expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenNthCalledWith(
      2,
      1,
      expect.any(AbortSignal),
    );
    await flushAsync();
    await flushAsync();
    expect(text("detail-name")).toBe("Alpha Group Policy Fresh");
  });

  it("keeps synced detail when an older detail refresh resolves afterwards", async () => {
    const refreshedDetail = deferred<UpstreamAccountDetail>();
    const sync = deferred<UpstreamAccountDetail>();
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockImplementationOnce(async () => refreshedDetail.promise)
      .mockResolvedValue(createDetail(1, "Alpha Synced"));
    apiMocks.syncUpstreamAccount.mockImplementationOnce(async () => sync.promise);

    render(<Probe />);
    await flushAsync();

    click("refresh");
    await flushAsync();
    click("sync-alpha");
    await flushAsync();

    sync.resolve(createDetail(1, "Alpha Synced"));
    await flushAsync();
    expect(text("detail-name")).toBe("Alpha Synced");

    refreshedDetail.resolve(createDetail(1, "Alpha Stale"));
    await flushAsync();
    expect(text("detail-name")).toBe("Alpha Synced");
  });

  it("does not clear the current account error when another account sync succeeds", async () => {
    const betaFailure = deferred<UpstreamAccountDetail>();
    const betaRefresh = deferred<UpstreamAccountDetail>();
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockImplementationOnce(async () => betaFailure.promise)
      .mockImplementationOnce(async () => betaRefresh.promise);
    apiMocks.syncUpstreamAccount.mockResolvedValueOnce(createDetail(1, "Alpha Synced"));

    render(<Probe />);
    await flushAsync();

    click("select-beta");
    await flushAsync();
    betaFailure.reject(new Error("Beta failed"));
    await flushAsync();
    expect(text("selected-id")).toBe("2");
    expect(text("error")).toBe("Beta failed");

    click("sync-alpha");
    await flushAsync();

    expect(text("selected-id")).toBe("2");
    expect(text("error")).toBe("Beta failed");
  });

  it("refreshes detail using the list's final selection", async () => {
    const betaDetail = deferred<UpstreamAccountDetail>();
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockResolvedValueOnce({
        writesEnabled: true,
        items: [createSummary(2, "Beta")],
        groups: [],
        hasUngroupedAccounts: false,
        routing: { writesEnabled: true, apiKeyConfigured: false, maskedApiKey: null },
      });
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockImplementationOnce(async (accountId: number) => {
        if (accountId !== 2) {
          throw new Error(`unexpected account ${accountId}`);
        }
        return betaDetail.promise;
      })
      .mockResolvedValue(createDetail(2, "Beta"));

    render(<Probe />);
    await flushAsync();

    click("refresh");
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenNthCalledWith(
      2,
      2,
      expect.any(AbortSignal),
    );

    betaDetail.resolve(createDetail(2, "Beta"));
    await flushAsync();

    expect(text("selected-id")).toBe("2");
    expect(text("detail-id")).toBe("2");
    expect(text("detail-name")).toBe("Beta");
    expect(text("error")).toBe("");
  });

  it("keeps the current detail when list refresh fails", async () => {
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockRejectedValueOnce(new Error("List failed"));
    apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));

    render(<Probe />);
    await flushAsync();

    expect(text("selected-id")).toBe("1");
    expect(text("detail-id")).toBe("1");
    expect(text("detail-name")).toBe("Alpha");

    click("refresh");
    await flushAsync();

    expect(text("selected-id")).toBe("1");
    expect(text("detail-id")).toBe("1");
    expect(text("detail-name")).toBe("Alpha");
    expect(text("error")).toBe("List failed");
  });

  it("keeps list and detail errors visible independently", async () => {
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockRejectedValueOnce(new Error("Beta failed"));
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockRejectedValueOnce(new Error("List failed"));

    render(<Probe />);
    await flushAsync();

    click("select-beta");
    await flushAsync();

    expect(text("detail-error")).toBe("Beta failed");
    expect(text("list-error")).toBe("");

    click("refresh");
    await flushAsync();

    expect(text("detail-error")).toBe("Beta failed");
    expect(text("list-error")).toBe("List failed");
  });

  it("keeps account detail errors scoped per account", async () => {
    apiMocks.fetchUpstreamAccountDetail
      .mockRejectedValueOnce(new Error("Alpha failed"))
      .mockRejectedValueOnce(new Error("Beta failed"))
      .mockRejectedValueOnce(new Error("Alpha failed"));

    render(<Probe />);
    await flushAsync();

    expect(text("selected-id")).toBe("1");
    expect(text("detail-error")).toBe("Alpha failed");

    click("select-beta");
    await flushAsync();
    expect(text("selected-id")).toBe("2");
    expect(text("detail-error")).toBe("Beta failed");

    click("select-alpha");
    await flushAsync();

    expect(text("selected-id")).toBe("1");
    expect(text("detail-error")).toBe("Alpha failed");
  });

  it("does not clear list errors after a non-list success", async () => {
    apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockRejectedValueOnce(new Error("List failed"));
    apiMocks.reloginUpstreamAccount.mockResolvedValueOnce({ loginId: "relogin-1" });

    render(<Probe />);
    await flushAsync();

    click("refresh");
    await flushAsync();
    expect(text("list-error")).toBe("List failed");

    click("relogin-alpha");
    await flushAsync();
    expect(text("list-error")).toBe("List failed");
  });

  it("clears an account error after that account sync succeeds off-selection", async () => {
    const sync = deferred<UpstreamAccountDetail>();
    apiMocks.fetchUpstreamAccountDetail
      .mockRejectedValueOnce(new Error("Alpha failed"))
      .mockResolvedValueOnce(createDetail(2, "Beta"))
      .mockResolvedValueOnce(createDetail(2, "Beta"));
    apiMocks.syncUpstreamAccount.mockImplementationOnce(async () => sync.promise);

    render(<Probe />);
    await flushAsync();

    expect(text("selected-id")).toBe("1");
    expect(text("detail-error")).toBe("Alpha failed");

    click("sync-alpha");
    click("select-beta");
    await flushAsync();

    sync.resolve(createDetail(1, "Alpha synced"));
    await flushAsync();

    click("select-alpha");
    await flushAsync();
    expect(text("selected-id")).toBe("1");
    expect(text("detail-error")).toBe("");
  });

  it("does not reclaim selection when a delete finishes after switching away", async () => {
    const remove = deferred<void>();
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockResolvedValueOnce(createDetail(2, "Beta"));
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce({
        writesEnabled: true,
        items: [createSummary(1, "Alpha"), createSummary(3, "Gamma"), createSummary(2, "Beta")],
        groups: [],
        hasUngroupedAccounts: false,
        routing: { writesEnabled: true, apiKeyConfigured: false, maskedApiKey: null },
      })
      .mockResolvedValueOnce({
        writesEnabled: true,
        items: [createSummary(2, "Beta"), createSummary(3, "Gamma")],
        groups: [],
        hasUngroupedAccounts: false,
        routing: { writesEnabled: true, apiKeyConfigured: false, maskedApiKey: null },
      });
    apiMocks.deleteUpstreamAccount.mockImplementationOnce(async () => remove.promise);

    render(<Probe />);
    await flushAsync();

    click("remove-alpha");
    click("select-beta");
    await flushAsync();

    remove.resolve();
    await flushAsync();

    expect(text("selected-id")).toBe("2");
    expect(text("selected-name")).toBe("Beta");
  });

  it("reanchors away from a deleted current account even if the list refresh fails", async () => {
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockRejectedValueOnce(new Error("List failed"));
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockResolvedValueOnce(createDetail(2, "Beta"));
    apiMocks.deleteUpstreamAccount.mockResolvedValueOnce();

    render(<Probe />);
    await flushAsync();

    expect(text("selected-id")).toBe("1");
    expect(text("detail-id")).toBe("1");

    click("remove-alpha");
    await flushAsync();
    await flushAsync();

    expect(text("selected-id")).toBe("2");
    expect(text("selected-name")).toBe("Beta");
    expect(text("detail-id")).not.toBe("1");
    expect(text("detail-name")).not.toBe("Alpha");
  });

  it("invalidates an older detail reload before sync refreshes the list", async () => {
    const refreshedDetail = deferred<UpstreamAccountDetail>();
    const syncedList = deferred<UpstreamAccountListResponse>();
    const sync = deferred<UpstreamAccountDetail>();

    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockResolvedValueOnce(createListResponse())
      .mockImplementationOnce(async () => syncedList.promise);
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockImplementationOnce(async () => refreshedDetail.promise)
      .mockResolvedValue(createDetail(1, "Alpha Synced"));
    apiMocks.syncUpstreamAccount.mockImplementationOnce(async () => sync.promise);

    render(<Probe />);
    await flushAsync();

    click("refresh");
    await flushAsync();
    click("sync-alpha");
    await flushAsync();

    sync.resolve(createDetail(1, "Alpha Synced"));
    await flushAsync();

    refreshedDetail.resolve(createDetail(1, "Alpha Stale"));
    await flushAsync();
    expect(text("detail-name")).toBe("Alpha");

    syncedList.resolve(createListResponse());
    await flushAsync();
    await flushAsync();
    expect(text("detail-name")).toBe("Alpha Synced");
  });

  it("refreshes the final selected account after switching during refresh", async () => {
    const refreshedList = deferred<UpstreamAccountListResponse>();
    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockImplementationOnce(async () => refreshedList.promise);
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockResolvedValueOnce(createDetail(2, "Beta Stale"))
      .mockResolvedValueOnce(createDetail(2, "Beta Fresh"));

    render(<Probe />);
    await flushAsync();

    click("refresh");
    click("select-beta");
    await flushAsync();

    expect(text("selected-id")).toBe("2");
    expect(text("detail-name")).toBe("Beta Stale");

    refreshedList.resolve(createListResponse());
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenNthCalledWith(
      3,
      2,
      expect.any(AbortSignal),
    );
    expect(text("selected-id")).toBe("2");
    expect(text("detail-id")).toBe("2");
    expect(text("detail-name")).toBe("Beta Fresh");
  });

  it("ignores an older list refresh after sync starts a newer list reload", async () => {
    const staleRefreshList = deferred<UpstreamAccountListResponse>();
    const syncedList = deferred<UpstreamAccountListResponse>();
    const sync = deferred<UpstreamAccountDetail>();

    apiMocks.fetchUpstreamAccounts
      .mockResolvedValueOnce(createListResponse())
      .mockImplementationOnce(async () => staleRefreshList.promise)
      .mockImplementationOnce(async () => syncedList.promise)
      .mockResolvedValue({
        writesEnabled: true,
        items: [createSummary(1, "Alpha Synced"), createSummary(2, "Beta")],
        groups: [],
        hasUngroupedAccounts: false,
        routing: { writesEnabled: true, apiKeyConfigured: false, maskedApiKey: null },
      });
    apiMocks.fetchUpstreamAccountDetail
      .mockResolvedValueOnce(createDetail(1, "Alpha"))
      .mockResolvedValue(createDetail(1, "Alpha Synced"));
    apiMocks.syncUpstreamAccount.mockImplementationOnce(async () => sync.promise);

    render(<Probe />);
    await flushAsync();

    click("refresh");
    await flushAsync();
    click("sync-alpha");
    await flushAsync();

    sync.resolve(createDetail(1, "Alpha Synced"));
    await flushAsync();

    syncedList.resolve({
      writesEnabled: true,
      items: [createSummary(1, "Alpha Synced"), createSummary(2, "Beta")],
      groups: [],
      hasUngroupedAccounts: false,
      routing: { writesEnabled: true, apiKeyConfigured: false, maskedApiKey: null },
    });
    await flushAsync();
    await flushAsync();

    expect(text("selected-name")).toBe("Alpha Synced");
    expect(text("detail-name")).toBe("Alpha Synced");

    staleRefreshList.resolve({
      writesEnabled: true,
      items: [createSummary(1, "Alpha Stale"), createSummary(2, "Beta")],
      groups: [],
      hasUngroupedAccounts: false,
      routing: { writesEnabled: true, apiKeyConfigured: false, maskedApiKey: null },
    });
    await flushAsync();

    expect(text("selected-id")).toBe("1");
    expect(text("selected-name")).toBe("Alpha Synced");
    expect(text("detail-name")).toBe("Alpha Synced");
  });

  it("silently refreshes the visible roster page after records SSE events", async () => {
    apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));

    render(<Probe />);
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(1);
    expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenCalledTimes(1);
    expect(text("detail-loading")).toBe("false");

    vi.useFakeTimers();
    try {
      act(() => {
        emitRecordsEvent();
        vi.advanceTimersByTime(UPSTREAM_ACCOUNTS_SSE_REFRESH_THROTTLE_MS);
      });
      await flushAsync();

      expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(2);
      expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenCalledTimes(2);
      expect(text("detail-loading")).toBe("false");
    } finally {
      vi.useRealTimers();
    }
  });

  it("skips an immediate open resync after a fresh load, then resyncs after the cooldown", async () => {
    apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));

    render(<Probe />);
    await flushAsync();

    expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(1);

    vi.useFakeTimers();
    try {
      act(() => {
        emitOpenEvent();
      });
      await flushAsync();
      expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(1);

      act(() => {
        vi.advanceTimersByTime(UPSTREAM_ACCOUNTS_OPEN_RESYNC_COOLDOWN_MS);
        emitOpenEvent();
      });
      await flushAsync();

      expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(2);
      expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenCalledTimes(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not let records SSE preempt the first roster load before the current query hydrates", async () => {
    const pendingList = deferred<UpstreamAccountListResponse>();
    apiMocks.fetchUpstreamAccounts.mockImplementationOnce(async () => pendingList.promise);
    apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));

    render(<Probe />);

    expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(1);

    vi.useFakeTimers();
    try {
      act(() => {
        emitRecordsEvent();
        vi.advanceTimersByTime(UPSTREAM_ACCOUNTS_SSE_REFRESH_THROTTLE_MS);
      });
      await flushAsync();

      expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(1);

      pendingList.resolve(createListResponse());
      await flushAsync();
      await flushAsync();

      expect(text("list-status")).toBe("ready");
      expect(text("selected-name")).toBe("Alpha");
      expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("coalesces same-query SSE refreshes into one follow-up request while a refresh is already running", async () => {
    const firstRefresh = deferred<UpstreamAccountListResponse>();
    const queuedRefresh = deferred<UpstreamAccountListResponse>();
    apiMocks.fetchUpstreamAccountDetail.mockResolvedValue(createDetail(1, "Alpha"));

    render(<Probe />);
    await flushAsync();

    apiMocks.fetchUpstreamAccounts
      .mockImplementationOnce(async () => firstRefresh.promise)
      .mockImplementationOnce(async () => queuedRefresh.promise);

    vi.useFakeTimers();
    try {
      act(() => {
        emitRecordsEvent();
        vi.advanceTimersByTime(UPSTREAM_ACCOUNTS_SSE_REFRESH_THROTTLE_MS);
      });
      await flushAsync();

      expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(2);

      act(() => {
        emitRecordsEvent();
        emitRecordsEvent();
        vi.advanceTimersByTime(UPSTREAM_ACCOUNTS_SSE_REFRESH_THROTTLE_MS);
      });
      await flushAsync();

      expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(2);

      firstRefresh.resolve(createListResponse({
        items: [createSummary(1, "Alpha Refresh 1"), createSummary(2, "Beta")],
      }));
      await flushAsync();
      await flushAsync();

      expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(3);

      queuedRefresh.resolve(createListResponse({
        items: [createSummary(1, "Alpha Refresh 2"), createSummary(2, "Beta")],
      }));
      await flushAsync();
      await flushAsync();

      expect(text("selected-name")).toBe("Alpha Refresh 2");
      expect(apiMocks.fetchUpstreamAccounts).toHaveBeenCalledTimes(3);
    } finally {
      vi.useRealTimers();
    }
  });
});
