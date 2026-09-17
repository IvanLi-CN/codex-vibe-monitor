/** @vitest-environment jsdom */
import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, vi } from "vitest";
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
import { useUpstreamAccounts } from "./useUpstreamAccounts";

const apiMocks = vi.hoisted(() => ({
  fetchUpstreamAccounts:
    vi.fn<(query?: FetchUpstreamAccountsQuery) => Promise<UpstreamAccountListResponse>>(),
  fetchUpstreamAccountDetail:
    vi.fn<
      (
        accountId: number,
        options?: { signal?: AbortSignal; includeRecentActions?: boolean } | AbortSignal,
      ) => Promise<UpstreamAccountDetail>
    >(),
  fetchUpstreamAccountWindowUsage:
    vi.fn<(accountIds: number[]) => Promise<UpstreamAccountWindowUsageResponse>>(),
  updateUpstreamAccountGroup:
    vi.fn<
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
  subscribeToSse: (listener: (payload: { type: string; records?: unknown[] }) => void) => {
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
function createSummary(id: number, displayName: string): UpstreamAccountSummary {
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
function createWindowedSummary(id: number, displayName: string): UpstreamAccountSummary {
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
function createForwardProxyNode(key: string, displayName = "JP Edge 01"): ForwardProxyBindingNode {
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
    routing: {
      writesEnabled: true,
      apiKeyConfigured: false,
      maskedApiKey: null,
    },
    ...overrides,
  };
}
type UpstreamAccountsState = ReturnType<typeof useUpstreamAccounts>;

function ProbeActions({
  selectedId,
  items,
  selectAccount,
  runSync,
  refresh,
  hydrateWindowUsage,
  loadDetail,
  beginRelogin,
  removeAccount,
  saveGroupNote,
}: Pick<
  UpstreamAccountsState,
  | "selectedId"
  | "items"
  | "selectAccount"
  | "runSync"
  | "refresh"
  | "hydrateWindowUsage"
  | "loadDetail"
  | "beginRelogin"
  | "removeAccount"
  | "saveGroupNote"
>): ReactNode {
  return (
    <>
      <button type="button" data-testid="select-beta" onClick={() => selectAccount(2)}>
        select beta
      </button>
      <button type="button" data-testid="select-alpha" onClick={() => selectAccount(1)}>
        select alpha
      </button>
      <button type="button" data-testid="select-gamma" onClick={() => selectAccount(3)}>
        select gamma
      </button>
      <button type="button" data-testid="sync-alpha" onClick={() => void runSync(1)}>
        sync alpha
      </button>
      <button type="button" data-testid="refresh" onClick={() => void refresh()}>
        refresh
      </button>
      <button
        type="button"
        data-testid="load-detail-recent-actions"
        onClick={() =>
          void loadDetail(selectedId ?? null, { silent: true, includeRecentActions: true })
        }
      >
        load detail recent actions
      </button>
      <button
        type="button"
        data-testid="hydrate-visible"
        onClick={() => void hydrateWindowUsage(items.map((item) => item.id))}
      >
        hydrate visible
      </button>
      <button type="button" data-testid="relogin-alpha" onClick={() => void beginRelogin(1)}>
        relogin alpha
      </button>
      <button type="button" data-testid="remove-alpha" onClick={() => void removeAccount(1)}>
        remove alpha
      </button>
      <button
        type="button"
        data-testid="save-prod-group"
        onClick={() => void saveGroupNote("prod", { routingRule: { priorityTier: "fallback" } })}
      >
        save prod group
      </button>
    </>
  );
}

function Probe({
  query,
  options,
}: {
  query?: FetchUpstreamAccountsQuery | null;
  options?: Parameters<typeof useUpstreamAccounts>[1];
}) {
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
    loadDetail,
    beginRelogin,
    removeAccount,
    saveGroupNote,
  } = useUpstreamAccounts(query, options);
  return (
    <div>
      <div data-testid="selected-id">{selectedId ?? ""}</div>
      <div data-testid="selected-name">{selectedSummary?.displayName ?? ""}</div>
      <div data-testid="detail-id">{detail?.id ?? ""}</div>
      <div data-testid="detail-name">{detail?.displayName ?? ""}</div>
      <div data-testid="detail-recent-actions-count">{detail?.recentActions?.length ?? 0}</div>
      <div data-testid="detail-loading">{isDetailLoading ? "true" : "false"}</div>
      <div data-testid="list-error">{listError ?? ""}</div>
      <div data-testid="list-freshness">{listState.freshness}</div>
      <div data-testid="list-loading-state">{listState.loadingState}</div>
      <div data-testid="list-status">{listState.status}</div>
      <div data-testid="list-has-current-query-data">
        {listState.hasCurrentQueryData ? "true" : "false"}
      </div>
      <div data-testid="proxy-catalog-kind">{forwardProxyCatalogState.kind}</div>
      <div data-testid="proxy-catalog-freshness">{forwardProxyCatalogState.freshness}</div>
      <div data-testid="window-usage-pending">{isWindowUsagePending ? "true" : "false"}</div>
      <div data-testid="first-item-id">{items[0]?.id ?? ""}</div>
      <div data-testid="first-item-primary-requests">
        {items[0]?.primaryWindow?.actualUsage?.requestCount ?? ""}
      </div>
      <div data-testid="first-item-secondary-requests">
        {items[0]?.secondaryWindow?.actualUsage?.requestCount ?? ""}
      </div>
      <div data-testid="detail-error">{detailError ?? ""}</div>
      <div data-testid="error">{error ?? ""}</div>
      <ProbeActions
        selectedId={selectedId}
        items={items}
        selectAccount={selectAccount}
        runSync={runSync}
        refresh={refresh}
        hydrateWindowUsage={hydrateWindowUsage}
        loadDetail={loadDetail}
        beginRelogin={beginRelogin}
        removeAccount={removeAccount}
        saveGroupNote={saveGroupNote}
      />
    </div>
  );
}

export {
  apiMocks,
  click,
  createDetail,
  createForwardProxyNode,
  createGroupSummary,
  createListResponse,
  createRateWindowActualUsage,
  createSummary,
  createWindowedSummary,
  createWindowUsageResponse,
  deferred,
  emitOpenEvent,
  emitRecordsEvent,
  flushAsync,
  host,
  Probe,
  render,
  rerender,
  root,
  sseMocks,
  text,
};
