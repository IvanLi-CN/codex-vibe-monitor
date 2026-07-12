/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import type {
  EffectiveRoutingRule,
  ForwardProxyBindingNode,
  UpstreamAccountGroupSummary,
  UpstreamAccountSummary,
} from "../../lib/api";
import AccountPoolLayout from "./AccountPoolLayout";
import GroupsPage from "./Groups";
import type { UpstreamAccountsLocationState } from "./UpstreamAccounts.shared-types";

type UpstreamAccountsHookValue = ReturnType<
  typeof import("../../hooks/useUpstreamAccounts").useUpstreamAccounts
>;

const hookMocks = vi.hoisted(() => ({
  useUpstreamAccounts: vi.fn(),
  useForwardProxyBindingNodes: vi.fn(),
}));
const storage = new Map<string, string>();

vi.mock("../../hooks/useUpstreamAccounts", () => ({
  useUpstreamAccounts: hookMocks.useUpstreamAccounts,
}));

vi.mock("../../hooks/useForwardProxyBindingNodes", () => ({
  useForwardProxyBindingNodes: hookMocks.useForwardProxyBindingNodes,
}));

const defaultEffectiveRoutingRule: EffectiveRoutingRule = {
  allowCutOut: true,
  allowCutIn: true,
  sourceTagIds: [],
  sourceTagNames: [],
};

function buildAccount(
  id: number,
  overrides: Partial<UpstreamAccountSummary>,
): UpstreamAccountSummary {
  return {
    id,
    kind: "oauth_codex",
    provider: "openai",
    displayName: `Test account ${id}`,
    groupName: "production",
    isMother: false,
    status: "active",
    displayStatus: "active",
    enabled: true,
    tags: [],
    effectiveRoutingRule: defaultEffectiveRoutingRule,
    ...overrides,
  };
}

const forwardProxyNodes: ForwardProxyBindingNode[] = [
  {
    key: "__direct__",
    source: "direct",
    displayName: "Direct",
    protocolLabel: "DIRECT",
    penalized: false,
    selectable: true,
    last24h: [],
  },
  {
    key: "jp-edge-01",
    source: "manual",
    displayName: "JP Edge 01",
    protocolLabel: "HTTP",
    penalized: false,
    selectable: true,
    last24h: [],
  },
];

function createHookValue(
  overrides?: Partial<UpstreamAccountsHookValue>,
): UpstreamAccountsHookValue {
  const items: UpstreamAccountSummary[] = [
    buildAccount(1, {
      displayName: "Production Team 01",
      groupName: "production",
      planType: "team",
      isMother: true,
    }),
    buildAccount(2, {
      displayName: "Production Team 02",
      groupName: "production",
      planType: "team",
    }),
    buildAccount(3, {
      kind: "api_key_codex",
      displayName: "Production API 01",
      groupName: "production",
      planType: "local",
    }),
    buildAccount(4, {
      displayName: "Staging Free 01",
      groupName: "staging",
      planType: "free",
    }),
    buildAccount(5, {
      displayName: "Ungrouped Rescue",
      groupName: null,
      planType: "free",
    }),
  ];

  const groups: UpstreamAccountGroupSummary[] = [
    {
      groupName: "production",
      note: "Premium traffic group.",
      boundProxyKeys: ["__direct__", "jp-edge-01"],
      concurrencyLimit: 6,
      nodeShuntEnabled: true,
      upstream429RetryEnabled: true,
      upstream429MaxRetries: 2,
    },
    {
      groupName: "staging",
      note: "Lower-risk staging traffic.",
      boundProxyKeys: ["jp-edge-01"],
      concurrencyLimit: 2,
      nodeShuntEnabled: false,
      upstream429RetryEnabled: false,
      upstream429MaxRetries: 0,
    },
  ];

  return {
    items,
    groups,
    forwardProxyNodes,
    forwardProxyCatalogState: {
      kind: "ready-with-data",
      freshness: "fresh",
      isPending: false,
      hasNodes: true,
    },
    hasUngroupedAccounts: true,
    writesEnabled: true,
    routing: null,
    selectedId: null,
    selectedSummary: null,
    detail: null,
    isLoading: false,
    isDetailLoading: false,
    listError: null,
    listState: {
      queryKey: '{"includeAll":true}',
      dataQueryKey: '{"includeAll":true}',
      freshness: "fresh",
      loadingState: "idle",
      status: "ready",
      hasCurrentQueryData: true,
      isPending: false,
    },
    isWindowUsagePending: false,
    detailError: null,
    error: null,
    missingDetailAccountId: null,
    selectAccount: vi.fn(),
    refresh: vi.fn(),
    hydrateWindowUsage: vi.fn(),
    loadDetail: vi.fn(),
    beginOauthLogin: vi.fn(),
    beginRelogin: vi.fn(),
    getLoginSession: vi.fn(),
    updateOauthLogin: vi.fn(),
    beginOauthMailboxSession: vi.fn(),
    beginOauthMailboxSessionForAddress: vi.fn(),
    getOauthMailboxStatuses: vi.fn(),
    removeOauthMailboxSession: vi.fn(),
    completeOauthLogin: vi.fn(),
    createApiKeyAccount: vi.fn(),
    runImportedOauthValidation: vi.fn(),
    startImportedOauthValidationJob: vi.fn(),
    stopImportedOauthValidationJob: vi.fn(),
    importOauthAccounts: vi.fn(),
    saveAccount: vi.fn(),
    saveRouting: vi.fn(),
    saveGroupNote: vi.fn(),
    runBulkAction: vi.fn(),
    startBulkSyncJob: vi.fn(),
    getBulkSyncJob: vi.fn(),
    stopBulkSyncJob: vi.fn(),
    runSync: vi.fn(),
    removeAccount: vi.fn(),
    total: items.length,
    page: 1,
    pageSize: 20,
    metrics: {
      total: items.length,
      oauth: 4,
      apiKey: 1,
      attention: 0,
    },
    ...overrides,
  };
}

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  class ResizeObserverMock {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  Object.defineProperty(globalThis, "ResizeObserver", {
    configurable: true,
    writable: true,
    value: ResizeObserverMock,
  });
  Object.defineProperty(window, "ResizeObserver", {
    configurable: true,
    writable: true,
    value: ResizeObserverMock,
  });
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: vi.fn().mockImplementation((query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  });
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: {
      getItem: vi.fn((key: string) => storage.get(key) ?? null),
      setItem: vi.fn((key: string, value: string) => {
        storage.set(key, value);
      }),
      removeItem: vi.fn((key: string) => {
        storage.delete(key);
      }),
      clear: vi.fn(() => {
        storage.clear();
      }),
    },
  });
});

beforeEach(() => {
  window.localStorage.setItem("codex-vibe-monitor.locale", "zh");
  hookMocks.useForwardProxyBindingNodes.mockReturnValue({
    nodes: forwardProxyNodes,
    error: null,
    isLoading: false,
    refresh: vi.fn(),
    catalogState: {
      kind: "ready-with-data",
      freshness: "fresh",
      isPending: false,
      hasNodes: true,
    },
  });
  hookMocks.useUpstreamAccounts.mockReturnValue(createHookValue());
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  vi.clearAllMocks();
  window.localStorage.clear();
});

function UpstreamAccountsStateEcho() {
  const location = useLocation();
  const state = location.state as UpstreamAccountsLocationState | null;
  return (
    <div data-testid="groups-test-state">
      {state?.presetGroupFilter
        ? `${state.presetGroupFilter.mode}:${state.presetGroupFilter.query}`
        : "none"}
    </div>
  );
}

function render(initialEntry = "/account-pool/groups") {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(
      <I18nProvider>
        <MemoryRouter initialEntries={[initialEntry]}>
          <Routes>
            <Route path="/account-pool" element={<AccountPoolLayout />}>
              <Route path="groups" element={<GroupsPage />} />
              <Route path="upstream-accounts" element={<UpstreamAccountsStateEcho />} />
              <Route
                path="upstream-accounts/new"
                element={<div data-testid="groups-test-create">create</div>}
              />
            </Route>
          </Routes>
        </MemoryRouter>
      </I18nProvider>,
    );
  });
}

async function flushAsync() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function findLink(text: string, rootNode: ParentNode = document.body) {
  return Array.from(rootNode.querySelectorAll("a")).find(
    (candidate) => candidate.textContent?.trim() === text,
  ) as HTMLAnchorElement | undefined;
}

function findButton(text: string, rootNode: ParentNode = document.body) {
  return Array.from(rootNode.querySelectorAll("button")).find(
    (candidate) => candidate.textContent?.trim() === text,
  ) as HTMLButtonElement | undefined;
}

describe("GroupsPage", () => {
  it("renders the /account-pool/groups route with the groups tab active", async () => {
    render();
    await flushAsync();

    expect(document.body.textContent).toContain("分组总览");
    expect(document.body.textContent).toContain("production");
    expect(document.body.textContent).toContain("Premium traffic group.");

    const activeLink = document.body.querySelector('a[href="/account-pool/groups"]');
    expect(activeLink?.getAttribute("aria-current")).toBe("page");
    expect(document.body.querySelectorAll('[data-testid="account-pool-group-row"]').length).toBe(2);
    expect(document.body.querySelector('[data-testid="account-pool-groups-list"]')).not.toBeNull();
  });

  it("opens the existing group settings dialog from the edit action", async () => {
    render();
    await flushAsync();

    const editButton = findButton("编辑分组设置");
    expect(editButton).toBeTruthy();

    act(() => {
      editButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushAsync();

    expect(document.body.textContent).toContain("分组设置");
    expect(document.body.textContent).toContain("production");
  });

  it("navigates to upstream accounts with a named preset group filter", async () => {
    render();
    await flushAsync();

    const viewAccountsLink = findLink("查看上游账号");
    expect(viewAccountsLink).toBeTruthy();

    act(() => {
      viewAccountsLink?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushAsync();

    expect(document.body.querySelector('[data-testid="groups-test-state"]')?.textContent).toBe(
      "exact:production",
    );
  });

  it("keeps the ungrouped card read-only and passes the ungrouped preset", async () => {
    hookMocks.useUpstreamAccounts.mockReturnValue(
      createHookValue({
        items: [
          buildAccount(21, {
            displayName: "Ungrouped OAuth Alpha",
            groupName: null,
            planType: "team",
          }),
          buildAccount(22, {
            kind: "api_key_codex",
            displayName: "Ungrouped API Key Beta",
            groupName: null,
            planType: "local",
          }),
        ],
        groups: [],
        total: 2,
        metrics: {
          total: 2,
          oauth: 1,
          apiKey: 1,
          attention: 0,
        },
      }),
    );

    render();
    await flushAsync();

    const ungroupedRow = document.body.querySelector(
      '[data-testid="account-pool-group-row-ungrouped"]',
    ) as HTMLElement | null;
    expect(ungroupedRow).not.toBeNull();
    expect(ungroupedRow?.textContent).toContain("未分组");
    expect(findButton("编辑分组设置", ungroupedRow ?? document.body)).toBeFalsy();

    const viewAccountsLink = findLink("查看上游账号", ungroupedRow ?? document.body);
    expect(viewAccountsLink).toBeTruthy();

    act(() => {
      viewAccountsLink?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushAsync();

    expect(document.body.querySelector('[data-testid="groups-test-state"]')?.textContent).toBe(
      "ungrouped:",
    );
  });
});
