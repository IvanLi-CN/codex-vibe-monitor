/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { MemoryRouter, Route, Routes, type InitialEntry } from "react-router-dom";
import { SystemNotificationProvider } from "../../components/ui/system-notifications";
import { I18nProvider } from "../../i18n";
import UpstreamAccountsPage from "./UpstreamAccounts";

const navigateMock = vi.hoisted(() => vi.fn());
const hookMocks = vi.hoisted(() => ({
  useUpstreamAccounts: vi.fn(),
  useUpstreamStickyConversations: vi.fn(),
}));

vi.mock("react-router-dom", async () => {
  const actual =
    await vi.importActual<typeof import("react-router-dom")>(
      "react-router-dom",
    );
  return {
    ...actual,
    useNavigate: () => navigateMock,
  };
});

vi.mock("../../hooks/useUpstreamAccounts", () => ({
  useUpstreamAccounts: hookMocks.useUpstreamAccounts,
}));

vi.mock("../../hooks/useUpstreamStickyConversations", () => ({
  useUpstreamStickyConversations: hookMocks.useUpstreamStickyConversations,
}));

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
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    writable: true,
    value: vi.fn(),
  });
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: {
      getItem: vi.fn((key: string) =>
        key === "codex-vibe-monitor.locale" ? "en" : null,
      ),
      setItem: vi.fn(),
      removeItem: vi.fn(),
    },
  });
});

beforeEach(() => {
  vi.mocked(window.localStorage.getItem).mockImplementation((key: string) =>
    key === "codex-vibe-monitor.locale" ? "en" : null,
  );
  hookMocks.useUpstreamStickyConversations.mockReturnValue({
    stats: null,
    isLoading: false,
    error: null,
  });
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  navigateMock.mockReset();
  vi.clearAllMocks();
});

function render(initialEntry: InitialEntry = "/account-pool/upstream-accounts") {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  rerender(initialEntry);
}

function rerender(initialEntry: InitialEntry = "/account-pool/upstream-accounts") {
  act(() => {
    root?.render(
      <I18nProvider>
        <SystemNotificationProvider>
          <MemoryRouter initialEntries={[initialEntry]}>
            <Routes>
              <Route
                path="/account-pool/upstream-accounts"
                element={<UpstreamAccountsPage />}
              />
            </Routes>
          </MemoryRouter>
        </SystemNotificationProvider>
      </I18nProvider>,
    );
  });
}

function findButton(pattern: RegExp) {
  return Array.from(document.body.querySelectorAll('button')).find((candidate) =>
    pattern.test(candidate.textContent || candidate.getAttribute('aria-label') || ''),
  ) as HTMLButtonElement | undefined
}

async function flushAsync() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function setInputValue(selector: string, value: string) {
  const input = document.body.querySelector(selector);
  if (
    !(input instanceof HTMLInputElement || input instanceof HTMLTextAreaElement)
  ) {
    throw new Error(`missing input: ${selector}`);
  }
  const prototype =
    input instanceof HTMLTextAreaElement
      ? HTMLTextAreaElement.prototype
      : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
  if (!setter) throw new Error(`missing native setter: ${selector}`);
  act(() => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

function clickButton(matcher: RegExp) {
  const button = Array.from(document.body.querySelectorAll("button")).find(
    (candidate) =>
      candidate instanceof HTMLButtonElement &&
      matcher.test(
        candidate.textContent ||
          candidate.getAttribute("aria-label") ||
          candidate.title ||
          "",
      ),
  );
  if (!(button instanceof HTMLButtonElement))
    throw new Error(`missing button: ${matcher}`);
  act(() => {
    button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  return button;
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

function mockAccountsPage() {
  hookMocks.useUpstreamAccounts.mockReturnValue({
    items: [
      {
        id: 5,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Existing OAuth",
        groupName: "prod",
        status: "active",
        enabled: true,
        duplicateInfo: {
          peerAccountIds: [9],
          reasons: ["sharedChatgptAccountId"],
        },
      },
      {
        id: 9,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Another OAuth",
        groupName: "prod",
        status: "active",
        enabled: true,
      },
    ],
    writesEnabled: true,
    selectedId: 5,
    selectedSummary: {
      id: 5,
      kind: "oauth_codex",
      provider: "codex",
      displayName: "Existing OAuth",
      groupName: "prod",
      status: "active",
      enabled: true,
      duplicateInfo: {
        peerAccountIds: [9],
        reasons: ["sharedChatgptAccountId"],
      },
    },
    detail: {
      id: 5,
      kind: "oauth_codex",
      provider: "codex",
      displayName: "Existing OAuth",
      groupName: "prod",
      status: "active",
      enabled: true,
      duplicateInfo: {
        peerAccountIds: [9],
        reasons: ["sharedChatgptAccountId"],
      },
      email: "dup@example.com",
      chatgptAccountId: "org_1",
      chatgptUserId: "user_1",
      history: [],
    },
    isLoading: false,
    isDetailLoading: false,
    listError: null,
    detailError: null,
    error: null,
    selectAccount: vi.fn(),
    refresh: vi.fn(),
    loadDetail: vi.fn(),
    beginOauthLogin: vi.fn(),
    beginRelogin: vi.fn(),
    getLoginSession: vi.fn(),
    completeOauthLogin: vi.fn(),
    createApiKeyAccount: vi.fn(),
    saveAccount: vi.fn(),
    saveRouting: vi.fn(),
    runSync: vi.fn(),
    removeAccount: vi.fn(),
    routing: { apiKeyConfigured: false, maskedApiKey: null },
  });
}

describe("UpstreamAccountsPage duplicates", () => {
  it("shows duplicate warnings in the roster and detail drawer", () => {
    mockAccountsPage();
    render("/account-pool/upstream-accounts");

    expect(document.body.textContent).toContain("Duplicate");
    clickButton(/Open details/i);
    expect(document.body.textContent).toContain(
      "Matched reasons: shared ChatGPT account id. Related account ids: 9.",
    );
  });

  it("clears the one-time duplicate warning after dismissing it", async () => {
    mockAccountsPage();
    render({
      pathname: "/account-pool/upstream-accounts",
      state: {
        selectedAccountId: 5,
        duplicateWarning: {
          accountId: 5,
          displayName: "Existing OAuth",
          peerAccountIds: [9],
          reasons: ["sharedChatgptAccountId"],
        },
      },
    });
    await flushAsync();

    expect(document.body.textContent).toContain(
      "was saved, but the upstream identity looks duplicated.",
    );
    clickButton(/Dismiss warning/i);
    expect(document.body.textContent).not.toContain(
      "was saved, but the upstream identity looks duplicated.",
    );
  });

  it("blocks saving when the edited display name conflicts with another account", () => {
    mockAccountsPage();
    render("/account-pool/upstream-accounts");

    clickButton(/Open details/i);
    setInputValue('input[name="detailDisplayName"]', " another oauth ");

    expect(document.body.textContent).toContain("Display name must be unique.");
    const saveButton = Array.from(
      document.body.querySelectorAll("button"),
    ).find((candidate) => /Save changes/i.test(candidate.textContent || ""));
    expect(saveButton).toBeInstanceOf(HTMLButtonElement);
    expect((saveButton as HTMLButtonElement).disabled).toBe(true);
  });

  it("keeps routing errors visible after an account action succeeds", async () => {
    const runSync = vi.fn().mockResolvedValue(undefined);
    const saveRouting = vi.fn().mockRejectedValue(new Error("Routing failed"));

    hookMocks.useUpstreamAccounts.mockReturnValue({
      items: [
        {
          id: 5,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Existing OAuth",
          groupName: "prod",
          status: "active",
          enabled: true,
        },
      ],
      writesEnabled: true,
      selectedId: 5,
      selectedSummary: {
        id: 5,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Existing OAuth",
        groupName: "prod",
        status: "active",
        enabled: true,
      },
      detail: {
        id: 5,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Existing OAuth",
        groupName: "prod",
        status: "active",
        enabled: true,
        email: "dup@example.com",
        history: [],
      },
      isLoading: false,
      isDetailLoading: false,
      listError: null,
      detailError: null,
      error: null,
      selectAccount: vi.fn(),
      refresh: vi.fn(),
      loadDetail: vi.fn(),
      beginOauthLogin: vi.fn(),
      beginRelogin: vi.fn(),
      getLoginSession: vi.fn(),
      completeOauthLogin: vi.fn(),
      createApiKeyAccount: vi.fn(),
      saveAccount: vi.fn(),
      saveRouting,
      runSync,
      removeAccount: vi.fn(),
      routing: { apiKeyConfigured: true, maskedApiKey: "pool-live••••" },
      groups: [],
    });
    hookMocks.useUpstreamStickyConversations.mockReturnValue({
      stats: { conversations: [], rangeStart: "", rangeEnd: "" },
      isLoading: false,
      error: null,
    });

    render();

    clickButton(/Edit pool key/i);
    clickButton(/Save pool key/i);
    await flushAsync();

    expect(document.body.textContent).toContain("Routing failed");

    clickButton(/Open details/i);
    clickButton(/Sync now/i);
    await flushAsync();

    expect(runSync).toHaveBeenCalledWith(5);
    expect(document.body.textContent).toContain("Routing failed");
  });

  it("renders list and detail errors independently", () => {
    hookMocks.useUpstreamAccounts.mockReturnValue({
      items: [
        {
          id: 5,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Existing OAuth",
          groupName: "prod",
          status: "active",
          enabled: true,
        },
      ],
      writesEnabled: true,
      selectedId: 5,
      selectedSummary: {
        id: 5,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Existing OAuth",
        groupName: "prod",
        status: "active",
        enabled: true,
      },
      detail: {
        id: 5,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Existing OAuth",
        groupName: "prod",
        status: "active",
        enabled: true,
        email: "dup@example.com",
        history: [],
      },
      isLoading: false,
      isDetailLoading: false,
      listError: "List failed",
      detailError: "Detail failed",
      error: "Detail failed",
      selectAccount: vi.fn(),
      refresh: vi.fn(),
      loadDetail: vi.fn(),
      beginOauthLogin: vi.fn(),
      beginRelogin: vi.fn(),
      getLoginSession: vi.fn(),
      completeOauthLogin: vi.fn(),
      createApiKeyAccount: vi.fn(),
      saveAccount: vi.fn(),
      saveRouting: vi.fn(),
      runSync: vi.fn(),
      removeAccount: vi.fn(),
      routing: { apiKeyConfigured: true, maskedApiKey: "pool-live••••" },
      groups: [],
    });
    hookMocks.useUpstreamStickyConversations.mockReturnValue({
      stats: { conversations: [], rangeStart: "", rangeEnd: "" },
      isLoading: false,
      error: null,
    });

    render();

    expect(document.body.textContent).toContain("List failed");
    expect(document.body.textContent).toContain("Detail failed");
  });
});

describe("UpstreamAccountsPage sync state isolation", () => {
  it("keeps another account's sync button idle while the previous account sync is still pending", async () => {
    const runSync = vi.fn().mockImplementation(
      () => new Promise(() => {}),
    );
    const selectAccount = vi.fn();
    const effectiveRoutingRule = {
      guardEnabled: false,
      lookbackHours: null,
      maxConversations: null,
      allowCutOut: false,
      allowCutIn: false,
      sourceTagIds: [],
      sourceTagNames: [],
      guardRules: [],
    };
    const baseState = {
      items: [
        {
          id: 5,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Existing OAuth",
          groupName: "prod",
          isMother: false,
          status: "active",
          enabled: true,
          tags: [],
          effectiveRoutingRule,
        },
        {
          id: 9,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Another OAuth",
          groupName: "prod",
          isMother: false,
          status: "active",
          enabled: true,
          tags: [],
          effectiveRoutingRule,
        },
      ],
      writesEnabled: true,
      isLoading: false,
      isDetailLoading: false,
      error: null,
      selectAccount,
      refresh: vi.fn(),
      loadDetail: vi.fn(),
      beginOauthLogin: vi.fn(),
      beginRelogin: vi.fn(),
      getLoginSession: vi.fn(),
      completeOauthLogin: vi.fn(),
      createApiKeyAccount: vi.fn(),
      saveAccount: vi.fn(),
      saveRouting: vi.fn(),
      runSync,
      removeAccount: vi.fn(),
      routing: { apiKeyConfigured: false, maskedApiKey: null },
      groups: [],
    };

    hookMocks.useUpstreamAccounts.mockReturnValue({
      ...baseState,
      selectedId: 5,
      selectedSummary: baseState.items[0],
      detail: {
        ...baseState.items[0],
        history: [],
      },
    });

    render();
    clickButton(/Open details/i);
    clickButton(/Sync now/i);
    expect(runSync).toHaveBeenCalledWith(5);

    hookMocks.useUpstreamAccounts.mockReturnValue({
      ...baseState,
      selectedId: 9,
      selectedSummary: baseState.items[1],
      detail: {
        ...baseState.items[1],
        history: [],
      },
    });
    rerender();
    await flushAsync();

    const syncButton = document.body.querySelector(
      '[data-testid="account-sync-button"]',
    );
    expect(syncButton?.querySelector(".animate-spin")).toBeNull();
    expect(
      syncButton?.querySelector('[data-icon-name="timer-refresh-outline"]'),
    ).not.toBeNull();
  });

  it("preserves the original account sync spinner after another account starts syncing", async () => {
    const runSync = vi.fn().mockImplementation(() => new Promise(() => {}));
    const effectiveRoutingRule = {
      guardEnabled: false,
      lookbackHours: null,
      maxConversations: null,
      allowCutOut: false,
      allowCutIn: false,
      sourceTagIds: [],
      sourceTagNames: [],
      guardRules: [],
    };
    const baseState = {
      items: [
        {
          id: 5,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Existing OAuth",
          groupName: "prod",
          isMother: false,
          status: "active",
          enabled: true,
          tags: [],
          effectiveRoutingRule,
        },
        {
          id: 9,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Another OAuth",
          groupName: "prod",
          isMother: false,
          status: "active",
          enabled: true,
          tags: [],
          effectiveRoutingRule,
        },
      ],
      writesEnabled: true,
      isLoading: false,
      isDetailLoading: false,
      error: null,
      selectAccount: vi.fn(),
      refresh: vi.fn(),
      loadDetail: vi.fn(),
      beginOauthLogin: vi.fn(),
      beginRelogin: vi.fn(),
      getLoginSession: vi.fn(),
      completeOauthLogin: vi.fn(),
      createApiKeyAccount: vi.fn(),
      saveAccount: vi.fn(),
      saveRouting: vi.fn(),
      runSync,
      removeAccount: vi.fn(),
      routing: { apiKeyConfigured: false, maskedApiKey: null },
      groups: [],
    };

    hookMocks.useUpstreamAccounts.mockReturnValue({
      ...baseState,
      selectedId: 5,
      selectedSummary: baseState.items[0],
      detail: {
        ...baseState.items[0],
        history: [],
      },
    });

    render();
    clickButton(/Open details/i);
    clickButton(/Sync now/i);

    hookMocks.useUpstreamAccounts.mockReturnValue({
      ...baseState,
      selectedId: 9,
      selectedSummary: baseState.items[1],
      detail: {
        ...baseState.items[1],
        history: [],
      },
    });
    rerender();
    await flushAsync();
    clickButton(/Sync now/i);

    hookMocks.useUpstreamAccounts.mockReturnValue({
      ...baseState,
      selectedId: 5,
      selectedSummary: baseState.items[0],
      detail: {
        ...baseState.items[0],
        history: [],
      },
    });
    rerender();
    await flushAsync();

    const syncButton = document.body.querySelector(
      '[data-testid="account-sync-button"]',
    ) as HTMLButtonElement | null;
    expect(runSync).toHaveBeenNthCalledWith(1, 5);
    expect(runSync).toHaveBeenNthCalledWith(2, 9);
    expect(syncButton?.disabled).toBe(true);
    expect(syncButton?.querySelector(".animate-spin")).not.toBeNull();
  });

  it("locks the whole account while one action is still pending", async () => {
    const saveAccount = vi.fn().mockImplementation(() => new Promise(() => {}));
    const effectiveRoutingRule = {
      guardEnabled: false,
      lookbackHours: null,
      maxConversations: null,
      allowCutOut: false,
      allowCutIn: false,
      sourceTagIds: [],
      sourceTagNames: [],
      guardRules: [],
    };
    const baseState = {
      items: [
        {
          id: 5,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Existing OAuth",
          groupName: "prod",
          isMother: false,
          status: "active",
          enabled: true,
          tags: [],
          effectiveRoutingRule,
        },
      ],
      writesEnabled: true,
      isLoading: false,
      isDetailLoading: false,
      error: null,
      selectAccount: vi.fn(),
      refresh: vi.fn(),
      loadDetail: vi.fn(),
      beginOauthLogin: vi.fn(),
      beginRelogin: vi.fn(),
      getLoginSession: vi.fn(),
      completeOauthLogin: vi.fn(),
      createApiKeyAccount: vi.fn(),
      saveAccount,
      saveRouting: vi.fn(),
      runSync: vi.fn(),
      removeAccount: vi.fn(),
      routing: { apiKeyConfigured: false, maskedApiKey: null },
      groups: [],
    };

    hookMocks.useUpstreamAccounts.mockReturnValue({
      ...baseState,
      selectedId: 5,
      selectedSummary: baseState.items[0],
      detail: {
        ...baseState.items[0],
        history: [],
      },
    });

    render();
    clickButton(/Open details/i);
    clickButton(/Save changes/i);

    const syncButton = document.body.querySelector(
      '[data-testid="account-sync-button"]',
    ) as HTMLButtonElement | null;
    const deleteButton = findButton(/Delete/i);

    expect(saveAccount).toHaveBeenCalledWith(5, expect.any(Object));
    expect(syncButton?.disabled).toBe(true);
    expect(deleteButton?.disabled).toBe(true);
  });

  it("does not show a stale sync error after switching to another account", async () => {
    const syncAlpha = deferred<void>();
    const runSync = vi.fn((accountId: number) =>
      accountId === 5 ? syncAlpha.promise : Promise.resolve(),
    );
    const effectiveRoutingRule = {
      guardEnabled: false,
      lookbackHours: null,
      maxConversations: null,
      allowCutOut: false,
      allowCutIn: false,
      sourceTagIds: [],
      sourceTagNames: [],
      guardRules: [],
    };
    const baseState = {
      items: [
        {
          id: 5,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Existing OAuth",
          groupName: "prod",
          isMother: false,
          status: "active",
          enabled: true,
          tags: [],
          effectiveRoutingRule,
        },
        {
          id: 9,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Another OAuth",
          groupName: "prod",
          isMother: false,
          status: "active",
          enabled: true,
          tags: [],
          effectiveRoutingRule,
        },
      ],
      writesEnabled: true,
      isLoading: false,
      isDetailLoading: false,
      error: null,
      selectAccount: vi.fn(),
      refresh: vi.fn(),
      loadDetail: vi.fn(),
      beginOauthLogin: vi.fn(),
      beginRelogin: vi.fn(),
      getLoginSession: vi.fn(),
      completeOauthLogin: vi.fn(),
      createApiKeyAccount: vi.fn(),
      saveAccount: vi.fn(),
      saveRouting: vi.fn(),
      runSync,
      removeAccount: vi.fn(),
      routing: { apiKeyConfigured: false, maskedApiKey: null },
      groups: [],
    };

    hookMocks.useUpstreamAccounts.mockReturnValue({
      ...baseState,
      selectedId: 5,
      selectedSummary: baseState.items[0],
      detail: {
        ...baseState.items[0],
        history: [],
      },
    });

    render();
    clickButton(/Open details/i);
    clickButton(/Sync now/i);

    hookMocks.useUpstreamAccounts.mockReturnValue({
      ...baseState,
      selectedId: 9,
      selectedSummary: baseState.items[1],
      detail: {
        ...baseState.items[1],
        history: [],
      },
    });
    rerender();
    await flushAsync();

    syncAlpha.reject(new Error("Alpha failed"));
    await flushAsync();

    expect(document.body.textContent).toContain("Another OAuth");
    expect(document.body.textContent).not.toContain("Alpha failed");
  });

  it("preserves one account's action error after another account starts a new action", async () => {
    const syncAlpha = deferred<void>();
    const runSync = vi.fn((accountId: number) =>
      accountId === 5 ? syncAlpha.promise : Promise.resolve(),
    );
    const effectiveRoutingRule = {
      guardEnabled: false,
      lookbackHours: null,
      maxConversations: null,
      allowCutOut: false,
      allowCutIn: false,
      sourceTagIds: [],
      sourceTagNames: [],
      guardRules: [],
    };
    const baseState = {
      items: [
        {
          id: 5,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Existing OAuth",
          groupName: "prod",
          isMother: false,
          status: "active",
          enabled: true,
          tags: [],
          effectiveRoutingRule,
        },
        {
          id: 9,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Another OAuth",
          groupName: "prod",
          isMother: false,
          status: "active",
          enabled: true,
          tags: [],
          effectiveRoutingRule,
        },
      ],
      writesEnabled: true,
      isLoading: false,
      isDetailLoading: false,
      error: null,
      selectAccount: vi.fn(),
      refresh: vi.fn(),
      loadDetail: vi.fn(),
      beginOauthLogin: vi.fn(),
      beginRelogin: vi.fn(),
      getLoginSession: vi.fn(),
      completeOauthLogin: vi.fn(),
      createApiKeyAccount: vi.fn(),
      saveAccount: vi.fn(),
      saveRouting: vi.fn(),
      runSync,
      removeAccount: vi.fn(),
      routing: { apiKeyConfigured: false, maskedApiKey: null },
      groups: [],
    };

    hookMocks.useUpstreamAccounts.mockReturnValue({
      ...baseState,
      selectedId: 5,
      selectedSummary: baseState.items[0],
      detail: {
        ...baseState.items[0],
        history: [],
      },
    });

    render();
    clickButton(/Open details/i);
    clickButton(/Sync now/i);
    syncAlpha.reject(new Error("Alpha failed"));
    await flushAsync();
    expect(document.body.textContent).toContain("Alpha failed");

    hookMocks.useUpstreamAccounts.mockReturnValue({
      ...baseState,
      selectedId: 9,
      selectedSummary: baseState.items[1],
      detail: {
        ...baseState.items[1],
        history: [],
      },
    });
    rerender();
    await flushAsync();
    clickButton(/Sync now/i);

    hookMocks.useUpstreamAccounts.mockReturnValue({
      ...baseState,
      selectedId: 5,
      selectedSummary: baseState.items[0],
      detail: {
        ...baseState.items[0],
        history: [],
      },
    });
    rerender();
    await flushAsync();

    expect(document.body.textContent).toContain("Alpha failed");
  });

  it("does not render stale detail content when the selected summary changed", async () => {
    const effectiveRoutingRule = {
      guardEnabled: false,
      lookbackHours: null,
      maxConversations: null,
      allowCutOut: false,
      allowCutIn: false,
      sourceTagIds: [],
      sourceTagNames: [],
      guardRules: [],
    };
    const staleDetail = {
      id: 5,
      kind: "oauth_codex",
      provider: "codex",
      displayName: "Existing OAuth",
      groupName: "prod",
      isMother: false,
      status: "active",
      enabled: true,
      email: "stale@example.com",
      tags: [],
      effectiveRoutingRule,
      history: [],
    };

    hookMocks.useUpstreamAccounts.mockReturnValue({
      items: [staleDetail, { ...staleDetail, id: 9, displayName: "Another OAuth" }],
      writesEnabled: true,
      selectedId: 9,
      selectedSummary: {
        ...staleDetail,
        id: 9,
        displayName: "Another OAuth",
        email: "fresh@example.com",
      },
      detail: staleDetail,
      isLoading: false,
      isDetailLoading: false,
      error: null,
      selectAccount: vi.fn(),
      refresh: vi.fn(),
      loadDetail: vi.fn(),
      beginOauthLogin: vi.fn(),
      beginRelogin: vi.fn(),
      getLoginSession: vi.fn(),
      completeOauthLogin: vi.fn(),
      createApiKeyAccount: vi.fn(),
      saveAccount: vi.fn(),
      saveRouting: vi.fn(),
      runSync: vi.fn(),
      removeAccount: vi.fn(),
      routing: { apiKeyConfigured: false, maskedApiKey: null },
      groups: [],
    });

    render();
    clickButton(/Open details/i);
    await flushAsync();

    expect(document.body.textContent).toContain("Another OAuth");
    expect(document.body.textContent).not.toContain("stale@example.com");
  });
});

describe("UpstreamAccountsPage api key details", () => {
  it("saves api key upstreamBaseUrl from the detail drawer", async () => {
    const saveAccount = vi.fn().mockResolvedValue({
      id: 8,
      kind: "api_key_codex",
      provider: "codex",
      displayName: "Gateway Key",
      groupName: "prod",
      isMother: false,
      status: "active",
      enabled: true,
      history: [],
      note: null,
      upstreamBaseUrl: "https://proxy.example.com/gateway",
      localLimits: {
        primaryLimit: 100,
        secondaryLimit: 1000,
        limitUnit: "requests",
      },
    });

    hookMocks.useUpstreamAccounts.mockReturnValue({
      items: [
        {
          id: 8,
          kind: "api_key_codex",
          provider: "codex",
          displayName: "Gateway Key",
          groupName: "prod",
          isMother: false,
          status: "active",
          enabled: true,
          maskedApiKey: "sk-gate••••",
        },
      ],
      writesEnabled: true,
      selectedId: 8,
      selectedSummary: {
        id: 8,
        kind: "api_key_codex",
        provider: "codex",
        displayName: "Gateway Key",
        groupName: "prod",
        isMother: false,
        status: "active",
        enabled: true,
        maskedApiKey: "sk-gate••••",
      },
      detail: {
        id: 8,
        kind: "api_key_codex",
        provider: "codex",
        displayName: "Gateway Key",
        groupName: "prod",
        isMother: false,
        status: "active",
        enabled: true,
        history: [],
        note: null,
        upstreamBaseUrl: "https://proxy.example.com/gateway",
        localLimits: {
          primaryLimit: 100,
          secondaryLimit: 1000,
          limitUnit: "requests",
        },
      },
      isLoading: false,
      isDetailLoading: false,
      error: null,
      selectAccount: vi.fn(),
      refresh: vi.fn(),
      loadDetail: vi.fn(),
      beginOauthLogin: vi.fn(),
      beginRelogin: vi.fn(),
      getLoginSession: vi.fn(),
      completeOauthLogin: vi.fn(),
      createApiKeyAccount: vi.fn(),
      saveAccount,
      saveRouting: vi.fn(),
      runSync: vi.fn(),
      removeAccount: vi.fn(),
      routing: { apiKeyConfigured: true, maskedApiKey: "pool-live••••" },
      groups: [],
    });
    hookMocks.useUpstreamStickyConversations.mockReturnValue({
      stats: { conversations: [], rangeStart: "", rangeEnd: "" },
      isLoading: false,
      error: null,
    });

    render();

    clickButton(/Open details/i);
    setInputValue(
      'input[name="detailUpstreamBaseUrl"]',
      "https://proxy.example.com/gateway/v2",
    );
    clickButton(/Save changes/i);
    await flushAsync();

    expect(saveAccount).toHaveBeenCalledWith(
      8,
      expect.objectContaining({
        upstreamBaseUrl: "https://proxy.example.com/gateway/v2",
      }),
    );
  });

  it("clears api key upstreamBaseUrl from the detail drawer with null payload", async () => {
    const saveAccount = vi.fn().mockResolvedValue({
      id: 8,
      kind: "api_key_codex",
      provider: "codex",
      displayName: "Gateway Key",
      groupName: "prod",
      isMother: false,
      status: "active",
      enabled: true,
      history: [],
      note: null,
      upstreamBaseUrl: null,
      localLimits: {
        primaryLimit: 100,
        secondaryLimit: 1000,
        limitUnit: "requests",
      },
    });

    hookMocks.useUpstreamAccounts.mockReturnValue({
      items: [
        {
          id: 8,
          kind: "api_key_codex",
          provider: "codex",
          displayName: "Gateway Key",
          groupName: "prod",
          isMother: false,
          status: "active",
          enabled: true,
          maskedApiKey: "sk-gate••••",
        },
      ],
      writesEnabled: true,
      selectedId: 8,
      selectedSummary: {
        id: 8,
        kind: "api_key_codex",
        provider: "codex",
        displayName: "Gateway Key",
        groupName: "prod",
        isMother: false,
        status: "active",
        enabled: true,
        maskedApiKey: "sk-gate••••",
      },
      detail: {
        id: 8,
        kind: "api_key_codex",
        provider: "codex",
        displayName: "Gateway Key",
        groupName: "prod",
        isMother: false,
        status: "active",
        enabled: true,
        history: [],
        note: null,
        upstreamBaseUrl: "https://proxy.example.com/gateway",
        localLimits: {
          primaryLimit: 100,
          secondaryLimit: 1000,
          limitUnit: "requests",
        },
      },
      isLoading: false,
      isDetailLoading: false,
      error: null,
      selectAccount: vi.fn(),
      refresh: vi.fn(),
      loadDetail: vi.fn(),
      beginOauthLogin: vi.fn(),
      beginRelogin: vi.fn(),
      getLoginSession: vi.fn(),
      completeOauthLogin: vi.fn(),
      createApiKeyAccount: vi.fn(),
      saveAccount,
      saveRouting: vi.fn(),
      runSync: vi.fn(),
      removeAccount: vi.fn(),
      routing: { apiKeyConfigured: true, maskedApiKey: "pool-live••••" },
      groups: [],
    });
    hookMocks.useUpstreamStickyConversations.mockReturnValue({
      stats: { conversations: [], rangeStart: "", rangeEnd: "" },
      isLoading: false,
      error: null,
    });

    render();

    clickButton(/Open details/i);
    setInputValue('input[name="detailUpstreamBaseUrl"]', "");
    clickButton(/Save changes/i);
    await flushAsync();

    expect(saveAccount).toHaveBeenCalledWith(
      8,
      expect.objectContaining({
        upstreamBaseUrl: null,
      }),
    );
  });

  it("blocks saving api key detail when upstreamBaseUrl includes a query string", () => {
    const saveAccount = vi.fn().mockResolvedValue({});

    hookMocks.useUpstreamAccounts.mockReturnValue({
      items: [
        {
          id: 8,
          kind: "api_key_codex",
          provider: "codex",
          displayName: "Gateway Key",
          groupName: "prod",
          isMother: false,
          status: "active",
          enabled: true,
          maskedApiKey: "sk-gate••••",
        },
      ],
      writesEnabled: true,
      selectedId: 8,
      selectedSummary: {
        id: 8,
        kind: "api_key_codex",
        provider: "codex",
        displayName: "Gateway Key",
        groupName: "prod",
        isMother: false,
        status: "active",
        enabled: true,
        maskedApiKey: "sk-gate••••",
      },
      detail: {
        id: 8,
        kind: "api_key_codex",
        provider: "codex",
        displayName: "Gateway Key",
        groupName: "prod",
        isMother: false,
        status: "active",
        enabled: true,
        history: [],
        note: null,
        upstreamBaseUrl: "https://proxy.example.com/gateway",
        localLimits: {
          primaryLimit: 100,
          secondaryLimit: 1000,
          limitUnit: "requests",
        },
      },
      isLoading: false,
      isDetailLoading: false,
      error: null,
      selectAccount: vi.fn(),
      refresh: vi.fn(),
      loadDetail: vi.fn(),
      beginOauthLogin: vi.fn(),
      beginRelogin: vi.fn(),
      getLoginSession: vi.fn(),
      completeOauthLogin: vi.fn(),
      createApiKeyAccount: vi.fn(),
      saveAccount,
      saveRouting: vi.fn(),
      runSync: vi.fn(),
      removeAccount: vi.fn(),
      routing: { apiKeyConfigured: true, maskedApiKey: "pool-live••••" },
      groups: [],
    });
    hookMocks.useUpstreamStickyConversations.mockReturnValue({
      stats: { conversations: [], rangeStart: "", rangeEnd: "" },
      isLoading: false,
      error: null,
    });

    render();

    clickButton(/Open details/i);
    setInputValue(
      'input[name="detailUpstreamBaseUrl"]',
      "https://proxy.example.com/gateway?team=prod",
    );

    expect(document.body.textContent).toContain(
      "cannot include a query string or fragment",
    );
    expect(findButton(/Save changes/i)?.disabled).toBe(true);
    expect(saveAccount).not.toHaveBeenCalled();
  });
});
