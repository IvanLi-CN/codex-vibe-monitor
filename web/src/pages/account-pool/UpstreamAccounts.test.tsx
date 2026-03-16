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

async function flushTimers() {
  await act(async () => {
    await new Promise((resolve) => window.setTimeout(resolve, 0));
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
  pressButton(button);
  return button;
}

function pressButton(button: HTMLButtonElement) {
  act(() => {
    if (typeof PointerEvent === "function") {
      button.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      button.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    }
    button.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    button.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
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
});

describe("UpstreamAccountsPage oauth recovery hints", () => {
  it("shows the bridge exchange hint for oauth accounts whose bridge token registration fails", () => {
    hookMocks.useUpstreamAccounts.mockReturnValue({
      items: [
        {
          id: 5,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Scope OAuth",
          groupName: "prod",
          isMother: false,
          status: "error",
          enabled: true,
          lastError:
            "oauth bridge token exchange failed: oauth bridge responded with 502",
        },
      ],
      writesEnabled: true,
      selectedId: 5,
      selectedSummary: {
        id: 5,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Scope OAuth",
        groupName: "prod",
        isMother: false,
        status: "error",
        enabled: true,
        lastError:
          "oauth bridge token exchange failed: oauth bridge responded with 502",
      },
      detail: {
        id: 5,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Scope OAuth",
        groupName: "prod",
        isMother: false,
        status: "error",
        enabled: true,
        lastError:
          "oauth bridge token exchange failed: oauth bridge responded with 502",
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
      groups: [],
    });

    render("/account-pool/upstream-accounts");

    clickButton(/Open details/i);
    expect(document.body.textContent).toContain(
      "This OAuth account could not register with the fixed bridge",
    );
    expect(document.body.textContent).toContain(
      "The built-in OAuth bridge rejected the refreshed access token exchange",
    );
    expect(document.body.textContent).toContain("Error");
    expect(document.body.textContent).not.toContain("Needs re-auth");
  });

  it("shows the re-auth hint only for explicit oauth invalidation", () => {
    hookMocks.useUpstreamAccounts.mockReturnValue({
      items: [
        {
          id: 6,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Expired OAuth",
          groupName: "prod",
          isMother: false,
          status: "needs_reauth",
          enabled: true,
          lastError:
            "OAuth token endpoint returned 400: invalid_grant",
        },
      ],
      writesEnabled: true,
      selectedId: 6,
      selectedSummary: {
        id: 6,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Expired OAuth",
        groupName: "prod",
        isMother: false,
        status: "needs_reauth",
        enabled: true,
        lastError:
          "OAuth token endpoint returned 400: invalid_grant",
      },
      detail: {
        id: 6,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Expired OAuth",
        groupName: "prod",
        isMother: false,
        status: "needs_reauth",
        enabled: true,
        lastError:
          "OAuth token endpoint returned 400: invalid_grant",
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
      groups: [],
    });

    render("/account-pool/upstream-accounts");

    clickButton(/Open details/i);
    expect(document.body.textContent).toContain(
      "This OAuth account needs a fresh sign-in",
    );
    expect(document.body.textContent).toContain("Needs re-auth");
  });

  it("prefers the bridge upstream hint over stale needs-reauth status when the last error is from the bridge data plane", () => {
    hookMocks.useUpstreamAccounts.mockReturnValue({
      items: [
        {
          id: 7,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Legacy Scope OAuth",
          groupName: "prod",
          isMother: false,
          status: "needs_reauth",
          enabled: true,
          lastError:
            "oauth bridge upstream rejected request: 403 forbidden",
        },
      ],
      writesEnabled: true,
      selectedId: 7,
      selectedSummary: {
        id: 7,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Legacy Scope OAuth",
        groupName: "prod",
        isMother: false,
        status: "needs_reauth",
        enabled: true,
        lastError:
          "oauth bridge upstream rejected request: 403 forbidden",
      },
      detail: {
        id: 7,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Legacy Scope OAuth",
        groupName: "prod",
        isMother: false,
        status: "needs_reauth",
        enabled: true,
        lastError:
          "oauth bridge upstream rejected request: 403 forbidden",
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
      groups: [],
    });

    render("/account-pool/upstream-accounts");

    clickButton(/Open details/i);
    expect(document.body.textContent).toContain(
      "The OAuth bridge upstream rejected this request",
    );
    expect(document.body.textContent).toContain("Error");
    expect(document.body.textContent).not.toContain(
      "This OAuth account needs a fresh sign-in",
    );
    expect(document.body.textContent).not.toContain("Needs re-auth");
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

describe("UpstreamAccountsPage delete confirmation", () => {
  it("opens an in-app confirmation bubble before deleting", async () => {
    const removeAccount = vi.fn().mockResolvedValue(undefined);
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);

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
        upstreamBaseUrl: null,
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
      saveAccount: vi.fn(),
      saveRouting: vi.fn(),
      runSync: vi.fn(),
      removeAccount,
      routing: { apiKeyConfigured: true, maskedApiKey: "pool-live••••" },
      groups: [],
      saveGroupNote: vi.fn(),
    });
    hookMocks.useUpstreamStickyConversations.mockReturnValue({
      stats: { conversations: [], rangeStart: "", rangeEnd: "" },
      isLoading: false,
      error: null,
    });

    render();

    clickButton(/Open details/i);
    const dialog = document.body.querySelector('[role="dialog"]');
    const deleteButton = dialog
      ? Array.from(dialog.querySelectorAll("button")).find((candidate) =>
          /^Delete$/i.test(
            candidate.textContent ||
              candidate.getAttribute("aria-label") ||
              candidate.title ||
              "",
          ),
        )
      : null;
    if (!(deleteButton instanceof HTMLButtonElement)) {
      throw new Error("missing detail drawer delete button");
    }
    pressButton(deleteButton);
    await flushAsync();
    await flushTimers();

    expect(removeAccount).not.toHaveBeenCalled();
    expect(confirmSpy).not.toHaveBeenCalled();
    const confirmDialog = document.body.querySelector('[role="alertdialog"]');
    expect(confirmDialog).not.toBeNull();
    expect(confirmDialog?.closest('[role="dialog"]')).not.toBeNull();
    expect(confirmDialog?.closest('.drawer-body')).toBeNull();
    const cancelButton = findButton(/Cancel/i);
    expect(cancelButton).toBeInstanceOf(HTMLButtonElement);
    expect(document.activeElement).toBe(cancelButton);
    const confirmDeleteButton = confirmDialog
      ? Array.from(confirmDialog.querySelectorAll('button')).find((candidate) =>
          /Delete account/i.test(
            candidate.textContent ||
              candidate.getAttribute('aria-label') ||
              candidate.title ||
              '',
          ),
        )
      : null;
    expect(confirmDeleteButton).toBeInstanceOf(HTMLButtonElement);

    clickButton(/Delete account/i);
    await flushAsync();

    expect(removeAccount).toHaveBeenCalledWith(8);
    expect(document.body.querySelector('[role="dialog"]')).toBeNull();
    confirmSpy.mockRestore();
  });

  it("keeps delete failures inside the detail drawer", async () => {
    const removeAccount = vi
      .fn()
      .mockRejectedValue(new Error("Request failed: 500 database is locked"));

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
        upstreamBaseUrl: null,
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
      saveAccount: vi.fn(),
      saveRouting: vi.fn(),
      runSync: vi.fn(),
      removeAccount,
      routing: { apiKeyConfigured: true, maskedApiKey: "pool-live••••" },
      groups: [],
      saveGroupNote: vi.fn(),
    });
    hookMocks.useUpstreamStickyConversations.mockReturnValue({
      stats: { conversations: [], rangeStart: "", rangeEnd: "" },
      isLoading: false,
      error: null,
    });

    render();

    clickButton(/Open details/i);
    const dialog = document.body.querySelector('[role="dialog"]');
    const deleteButton = dialog
      ? Array.from(dialog.querySelectorAll("button")).find((candidate) =>
          /^Delete$/i.test(
            candidate.textContent ||
              candidate.getAttribute("aria-label") ||
              candidate.title ||
              "",
          ),
        )
      : null;
    if (!(deleteButton instanceof HTMLButtonElement)) {
      throw new Error("missing detail drawer delete button");
    }
    pressButton(deleteButton);
    clickButton(/Delete account/i);
    await flushAsync();

    const matchingStatuses = Array.from(
      document.body.querySelectorAll('[role="status"]'),
    ).filter((node) =>
      (node.textContent || "").includes("Request failed: 500 database is locked"),
    );
    expect(matchingStatuses).toHaveLength(1);
    expect(matchingStatuses[0]?.closest('[role="dialog"]')).not.toBeNull();
  });

  it("keeps delete failures visible when the detail payload is unavailable", async () => {
    const removeAccount = vi
      .fn()
      .mockRejectedValue(new Error("Request failed: 500 database is locked"));

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
      detail: null,
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
      removeAccount,
      routing: { apiKeyConfigured: true, maskedApiKey: "pool-live••••" },
      groups: [],
      saveGroupNote: vi.fn(),
    });
    hookMocks.useUpstreamStickyConversations.mockReturnValue({
      stats: { conversations: [], rangeStart: "", rangeEnd: "" },
      isLoading: false,
      error: null,
    });

    render();

    clickButton(/Open details/i);
    const dialog = document.body.querySelector('[role="dialog"]');
    const deleteButton = dialog
      ? Array.from(dialog.querySelectorAll("button")).find((candidate) =>
          /^Delete$/i.test(
            candidate.textContent ||
              candidate.getAttribute("aria-label") ||
              candidate.title ||
              "",
          ),
        )
      : null;
    if (!(deleteButton instanceof HTMLButtonElement)) {
      throw new Error("missing detail drawer delete button");
    }

    pressButton(deleteButton);
    clickButton(/Delete account/i);
    await flushAsync();

    const matchingStatuses = Array.from(
      document.body.querySelectorAll('[role="status"]'),
    ).filter((node) =>
      (node.textContent || "").includes("Request failed: 500 database is locked"),
    );
    expect(matchingStatuses).toHaveLength(1);
    expect(matchingStatuses[0]?.closest('[role="dialog"]')).not.toBeNull();
  });

  it("closes only the delete confirmation on escape", async () => {
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
        upstreamBaseUrl: null,
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
      saveAccount: vi.fn(),
      saveRouting: vi.fn(),
      runSync: vi.fn(),
      removeAccount: vi.fn(),
      routing: { apiKeyConfigured: true, maskedApiKey: "pool-live••••" },
      groups: [],
      saveGroupNote: vi.fn(),
    });
    hookMocks.useUpstreamStickyConversations.mockReturnValue({
      stats: { conversations: [], rangeStart: "", rangeEnd: "" },
      isLoading: false,
      error: null,
    });

    render();

    clickButton(/Open details/i);
    clickButton(/^Delete$/i);
    await flushAsync();
    await flushTimers();

    expect(document.body.querySelector('[role="alertdialog"]')).not.toBeNull();

    act(() => {
      document.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
    });
    await flushAsync();

    expect(document.body.querySelector('[role="alertdialog"]')).toBeNull();
    expect(document.body.querySelector('[role="dialog"]')).not.toBeNull();
  });

  it("keeps an initially opened delete confirmation inside the drawer dialog subtree", async () => {
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
        upstreamBaseUrl: null,
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
      saveAccount: vi.fn(),
      saveRouting: vi.fn(),
      runSync: vi.fn(),
      removeAccount: vi.fn(),
      routing: { apiKeyConfigured: true, maskedApiKey: "pool-live••••" },
      groups: [],
      saveGroupNote: vi.fn(),
    });
    hookMocks.useUpstreamStickyConversations.mockReturnValue({
      stats: { conversations: [], rangeStart: "", rangeEnd: "" },
      isLoading: false,
      error: null,
    });

    render({
      pathname: "/account-pool/upstream-accounts",
      state: {
        selectedAccountId: 8,
        openDetail: true,
        openDeleteConfirm: true,
      },
    });
    await flushAsync();
    await flushTimers();

    const confirmDialog = document.body.querySelector('[role="alertdialog"]');
    expect(confirmDialog).not.toBeNull();
    expect(confirmDialog?.closest('[role="dialog"]')).not.toBeNull();
    expect(confirmDialog?.closest('.drawer-body')).toBeNull();
  });

  it("keeps the drawer open until a failing delete request settles", async () => {
    let rejectRemove: ((reason?: unknown) => void) | null = null;
    const removeAccount = vi.fn(
      () =>
        new Promise<void>((_, reject) => {
          rejectRemove = reject;
        }),
    );

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
        upstreamBaseUrl: null,
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
      saveAccount: vi.fn(),
      saveRouting: vi.fn(),
      runSync: vi.fn(),
      removeAccount,
      routing: { apiKeyConfigured: true, maskedApiKey: "pool-live••••" },
      groups: [],
      saveGroupNote: vi.fn(),
    });
    hookMocks.useUpstreamStickyConversations.mockReturnValue({
      stats: { conversations: [], rangeStart: "", rangeEnd: "" },
      isLoading: false,
      error: null,
    });

    render();

    clickButton(/Open details/i);
    const dialog = document.body.querySelector('[role="dialog"]');
    const deleteButton = dialog
      ? Array.from(dialog.querySelectorAll("button")).find((candidate) =>
          /^Delete$/i.test(
            candidate.textContent ||
              candidate.getAttribute("aria-label") ||
              candidate.title ||
              "",
          ),
        )
      : null;
    if (!(deleteButton instanceof HTMLButtonElement)) {
      throw new Error("missing detail drawer delete button");
    }

    pressButton(deleteButton);
    clickButton(/Delete account/i);
    await flushAsync();

    act(() => {
      document.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
    });
    await flushAsync();

    expect(document.body.querySelector('[role="dialog"]')).not.toBeNull();

    act(() => {
      rejectRemove?.(new Error("Request failed: 500 database is locked"));
    });
    await flushAsync();

    const matchingStatuses = Array.from(
      document.body.querySelectorAll('[role="status"]'),
    ).filter((node) =>
      (node.textContent || "").includes("Request failed: 500 database is locked"),
    );
    expect(matchingStatuses).toHaveLength(1);
    expect(matchingStatuses[0]?.closest('[role="dialog"]')).not.toBeNull();
  });
});
