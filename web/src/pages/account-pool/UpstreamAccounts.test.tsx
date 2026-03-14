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
import { MemoryRouter, Route, Routes } from "react-router-dom";
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

function render(initialEntry = "/account-pool/upstream-accounts") {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(
      <I18nProvider>
        <MemoryRouter initialEntries={[initialEntry]}>
          <Routes>
            <Route
              path="/account-pool/upstream-accounts"
              element={<UpstreamAccountsPage />}
            />
          </Routes>
        </MemoryRouter>
      </I18nProvider>,
    );
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
