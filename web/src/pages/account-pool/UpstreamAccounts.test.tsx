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
import type { EffectiveRoutingRule } from "../../lib/api";

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

const defaultEffectiveRoutingRule: EffectiveRoutingRule = {
  guardEnabled: false,
  lookbackHours: null,
  maxConversations: null,
  allowCutOut: true,
  allowCutIn: true,
  sourceTagIds: [],
  sourceTagNames: [],
  guardRules: [],
};

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
        isMother: true,
        planType: "team",
        primaryWindow: {
          usedPercent: 42,
          usedText: "42 requests",
          limitText: "120 requests",
          resetsAt: "2026-03-16T06:55:00.000Z",
          windowDurationMins: 300,
        },
        secondaryWindow: {
          usedPercent: 12,
          usedText: "12 requests",
          limitText: "500 requests",
          resetsAt: "2026-03-18T00:00:00.000Z",
          windowDurationMins: 10080,
        },
        credits: null,
        localLimits: null,
        duplicateInfo: {
          peerAccountIds: [9],
          reasons: ["sharedChatgptAccountId"],
        },
        tags: [
          { id: 1, name: "vip", routingRule: defaultEffectiveRoutingRule },
          { id: 2, name: "burst-safe", routingRule: defaultEffectiveRoutingRule },
          { id: 3, name: "prod-apac", routingRule: defaultEffectiveRoutingRule },
          { id: 4, name: "sticky-pool", routingRule: defaultEffectiveRoutingRule },
        ],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
      {
        id: 9,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Another OAuth",
        groupName: "prod",
        status: "active",
        enabled: true,
        isMother: false,
        planType: "pro",
        primaryWindow: null,
        secondaryWindow: null,
        credits: null,
        localLimits: null,
        tags: [],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
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
      isMother: true,
      planType: "team",
      lastSuccessfulSyncAt: "2026-03-16T01:55:00.000Z",
      lastActivityAt: "2026-03-16T02:05:00.000Z",
      primaryWindow: {
        usedPercent: 42,
        usedText: "42 requests",
        limitText: "120 requests",
        resetsAt: "2026-03-16T06:55:00.000Z",
        windowDurationMins: 300,
      },
      secondaryWindow: {
        usedPercent: 12,
        usedText: "12 requests",
        limitText: "500 requests",
        resetsAt: "2026-03-18T00:00:00.000Z",
        windowDurationMins: 10080,
      },
      credits: null,
      localLimits: null,
      duplicateInfo: {
        peerAccountIds: [9],
        reasons: ["sharedChatgptAccountId"],
      },
      tags: [
        { id: 1, name: "vip", routingRule: defaultEffectiveRoutingRule },
        { id: 2, name: "burst-safe", routingRule: defaultEffectiveRoutingRule },
        { id: 3, name: "prod-apac", routingRule: defaultEffectiveRoutingRule },
        { id: 4, name: "sticky-pool", routingRule: defaultEffectiveRoutingRule },
      ],
      effectiveRoutingRule: defaultEffectiveRoutingRule,
    },
    detail: {
      id: 5,
      kind: "oauth_codex",
      provider: "codex",
      displayName: "Existing OAuth",
      groupName: "prod",
      status: "active",
      enabled: true,
      isMother: true,
      planType: "team",
      lastSuccessfulSyncAt: "2026-03-16T01:55:00.000Z",
      lastActivityAt: "2026-03-16T02:05:00.000Z",
      primaryWindow: {
        usedPercent: 42,
        usedText: "42 requests",
        limitText: "120 requests",
        resetsAt: "2026-03-16T06:55:00.000Z",
        windowDurationMins: 300,
      },
      secondaryWindow: {
        usedPercent: 12,
        usedText: "12 requests",
        limitText: "500 requests",
        resetsAt: "2026-03-18T00:00:00.000Z",
        windowDurationMins: 10080,
      },
      duplicateInfo: {
        peerAccountIds: [9],
        reasons: ["sharedChatgptAccountId"],
      },
      email: "dup@example.com",
      chatgptAccountId: "org_1",
      chatgptUserId: "user_1",
      credits: null,
      localLimits: null,
      tags: [
        { id: 1, name: "vip", routingRule: defaultEffectiveRoutingRule },
        { id: 2, name: "burst-safe", routingRule: defaultEffectiveRoutingRule },
        { id: 3, name: "prod-apac", routingRule: defaultEffectiveRoutingRule },
        { id: 4, name: "sticky-pool", routingRule: defaultEffectiveRoutingRule },
      ],
      effectiveRoutingRule: defaultEffectiveRoutingRule,
      history: [],
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
    saveAccount: vi.fn(),
    saveRouting: vi.fn(),
    runSync: vi.fn(),
    removeAccount: vi.fn(),
    routing: { apiKeyConfigured: false, maskedApiKey: null },
  });
}

describe("UpstreamAccountsPage duplicates", () => {
  it("renders the compact roster header and folded metadata chips", () => {
    mockAccountsPage();
    render("/account-pool/upstream-accounts");

    const headerCells = Array.from(document.body.querySelectorAll("thead th")).map((cell) =>
      cell.textContent?.trim() || "",
    );
    expect(headerCells).toEqual(["Account", "Sync / Call", "Windows", ""]);
    expect(document.body.textContent).toContain("vip");
    expect(document.body.textContent).toContain("+2");
    expect(document.body.textContent).toContain("team");
  });

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
