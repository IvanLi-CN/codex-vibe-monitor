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
import { SystemNotificationProvider } from "../../components/ui/system-notifications";
import { I18nProvider } from "../../i18n";
import UpstreamAccountCreatePage from "./UpstreamAccountCreate";

const navigateMock = vi.hoisted(() => vi.fn());
const hookMocks = vi.hoisted(() => ({
  useUpstreamAccounts: vi.fn(),
}));
const apiMocks = vi.hoisted(() => ({
  fetchUpstreamAccountDetail: vi.fn(),
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

vi.mock("../../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../../lib/api")>(
    "../../lib/api",
  );
  return {
    ...actual,
    fetchUpstreamAccountDetail: apiMocks.fetchUpstreamAccountDetail,
  };
});

type RenderEntry =
  | string
  | {
      pathname: string;
      search?: string;
      state?: unknown;
    };

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
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  navigateMock.mockReset();
  apiMocks.fetchUpstreamAccountDetail.mockReset();
  vi.clearAllMocks();
});

function render(initialEntry: RenderEntry = "/account-pool/upstream-accounts/new") {
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
                path="/account-pool/upstream-accounts/new"
                element={<UpstreamAccountCreatePage />}
              />
            </Routes>
          </MemoryRouter>
        </SystemNotificationProvider>
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

async function flushTimers() {
  await act(async () => {
    await new Promise((resolve) => window.setTimeout(resolve, 0));
  });
}

function setInputValue(selector: string, value: string) {
  const input = host?.querySelector(selector);
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
  if (!setter) {
    throw new Error(`missing native setter: ${selector}`);
  }
  act(() => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
  return input;
}

function setFieldValue(
  input: HTMLInputElement | HTMLTextAreaElement,
  value: string,
) {
  const prototype =
    input instanceof HTMLTextAreaElement
      ? HTMLTextAreaElement.prototype
      : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
  if (!setter) {
    throw new Error("missing native setter");
  }
  act(() => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
  return input;
}

function setBodyInputValue(selector: string, value: string) {
  const input = document.body.querySelector(selector);
  if (
    !(input instanceof HTMLInputElement || input instanceof HTMLTextAreaElement)
  ) {
    throw new Error(`missing body input: ${selector}`);
  }
  return setFieldValue(input, value);
}

function clickButton(matcher: RegExp) {
  const button = Array.from(host?.querySelectorAll("button") ?? []).find(
    (candidate) =>
      candidate instanceof HTMLButtonElement &&
      matcher.test(
        [
          candidate.textContent,
          candidate.getAttribute("aria-label"),
          candidate.title,
        ]
          .filter(Boolean)
          .join(" "),
      ),
  );
  if (!(button instanceof HTMLButtonElement)) {
    throw new Error(`missing button: ${matcher}`);
  }
  act(() => {
    button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  return button;
}

function clickBodyButton(matcher: RegExp) {
  const button = Array.from(document.body.querySelectorAll("button")).find(
    (candidate) =>
      candidate instanceof HTMLButtonElement &&
      matcher.test(
        [
          candidate.textContent,
          candidate.getAttribute("aria-label"),
          candidate.title,
        ]
          .filter(Boolean)
          .join(" "),
      ),
  );
  if (!(button instanceof HTMLButtonElement)) {
    throw new Error(`missing button: ${matcher}`);
  }
  act(() => {
    button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  return button;
}

function findButton(matcher: RegExp) {
  return Array.from(host?.querySelectorAll("button") ?? []).find(
    (candidate) =>
      candidate instanceof HTMLButtonElement &&
      matcher.test(
        [
          candidate.textContent,
          candidate.getAttribute("aria-label"),
          candidate.title,
        ]
          .filter(Boolean)
          .join(" "),
      ),
  ) as HTMLButtonElement | undefined;
}

function findBodyButton(matcher: RegExp) {
  return Array.from(document.body.querySelectorAll("button")).find(
    (candidate) =>
      candidate instanceof HTMLButtonElement &&
      matcher.test(
        [
          candidate.textContent,
          candidate.getAttribute("aria-label"),
          candidate.title,
        ]
          .filter(Boolean)
          .join(" "),
      ),
  ) as HTMLButtonElement | undefined;
}

function getBatchRows() {
  return host?.querySelectorAll('[data-testid^="batch-oauth-row-"]') ?? [];
}

function setComboboxValue(nameSelector: string, value: string) {
  const hiddenInput = host?.querySelector(nameSelector);
  if (!(hiddenInput instanceof HTMLInputElement)) {
    throw new Error(`missing combobox input: ${nameSelector}`);
  }
  const wrapper = hiddenInput.parentElement;
  const trigger = wrapper?.querySelector('button[role="combobox"]');
  if (!(trigger instanceof HTMLButtonElement)) {
    throw new Error(`missing combobox trigger: ${nameSelector}`);
  }
  act(() => {
    trigger.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });

  const searchInput = document.body.querySelector("[cmdk-input]");
  if (!(searchInput instanceof HTMLInputElement)) {
    throw new Error(`missing command input: ${nameSelector}`);
  }
  const setter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    "value",
  )?.set;
  if (!setter) {
    throw new Error(`missing native setter for combobox: ${nameSelector}`);
  }
  act(() => {
    setter.call(searchInput, value);
    searchInput.dispatchEvent(new Event("input", { bubbles: true }));
    searchInput.dispatchEvent(new Event("change", { bubbles: true }));
  });

  const option = Array.from(document.body.querySelectorAll("[cmdk-item]")).find(
    (candidate) => (candidate.textContent || "").includes(value),
  );
  if (!(option instanceof HTMLElement)) {
    throw new Error(`missing combobox option: ${value}`);
  }
  act(() => {
    option.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

function mockUpstreamAccounts(
  overrides: Partial<ReturnType<typeof hookMocks.useUpstreamAccounts>> = {},
) {
  apiMocks.fetchUpstreamAccountDetail.mockResolvedValue({
    id: 41,
    kind: "oauth_codex",
    provider: "codex",
    displayName: "Row One",
    groupName: "prod",
    status: "active",
    enabled: true,
    duplicateInfo: null,
    history: [],
  });
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
    groups: [],
    writesEnabled: true,
    isLoading: false,
    listError: null,
    detailError: null,
    error: null,
    beginOauthLogin: vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    }),
    getLoginSession: vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    }),
    completeOauthLogin: vi
      .fn()
      .mockResolvedValue({ id: 41, displayName: "Row One" }),
    beginOauthMailboxSession: vi.fn().mockResolvedValue({
      supported: true,
      sessionId: "mailbox-1",
      emailAddress: "mailbox-1@example.com",
      expiresAt: "2026-04-13T10:00:00.000Z",
      source: "generated",
    }),
    beginOauthMailboxSessionForAddress: vi.fn().mockResolvedValue({
      supported: true,
      sessionId: "mailbox-attached-1",
      emailAddress: "mailbox-1@example.com",
      expiresAt: "2026-04-13T10:00:00.000Z",
      source: "attached",
    }),
    getOauthMailboxStatuses: vi.fn().mockResolvedValue([]),
    removeOauthMailboxSession: vi.fn().mockResolvedValue(undefined),
    createApiKeyAccount: vi.fn(),
    saveGroupNote: vi.fn().mockResolvedValue({ groupName: "prod", note: "Saved note" }),
    ...overrides,
  });
}

describe("UpstreamAccountCreatePage batch oauth", () => {
  it("does not show background detail errors as the create-page banner", () => {
    mockUpstreamAccounts({ detailError: "Background detail failed", error: "Background detail failed" });
    render("/account-pool/upstream-accounts/new?mode=batchOauth");

    expect(document.body.textContent).not.toContain("Background detail failed");
  });

  it("opens batch oauth mode from the query string with five empty rows", () => {
    mockUpstreamAccounts();
    render("/account-pool/upstream-accounts/new?mode=batchOauth");

    expect(
      Array.from(host?.querySelectorAll('[role="tab"]') ?? []).some((tab) =>
        /Batch OAuth/.test(tab.textContent ?? ""),
      ),
    ).toBe(true);
    expect(getBatchRows()).toHaveLength(5);
    expect(host?.textContent).toContain("Batch Codex OAuth onboarding");
  });

  it("updates the query-backed mode when tabs change", () => {
    mockUpstreamAccounts();
    render("/account-pool/upstream-accounts/new?mode=batchOauth");

    clickButton(/^API key$/i);

    expect(navigateMock).toHaveBeenCalledWith(
      "/account-pool/upstream-accounts/new?mode=apiKey",
      {
        replace: true,
      },
    );
    expect(host?.textContent).toContain("Codex API key account");
  });

  it("forces relink flows back to single oauth mode", () => {
    mockUpstreamAccounts();
    render("/account-pool/upstream-accounts/new?accountId=5&mode=batchOauth");

    expect(host?.textContent).toContain("Re-authorize upstream account");
    expect(host?.textContent).not.toContain("Batch OAuth");

    const displayNameInput = host?.querySelector(
      'input[name="oauthDisplayName"]',
    );
    expect(displayNameInput).toBeInstanceOf(HTMLInputElement);
    expect((displayNameInput as HTMLInputElement).value).toBe("Existing OAuth");
  });

  it("applies the header default group to existing blank rows and new rows", async () => {
    mockUpstreamAccounts();
    render("/account-pool/upstream-accounts/new?mode=batchOauth");

    setComboboxValue('input[name="batchOauthDefaultGroupName"]', "prod");
    await flushAsync();

    const groupInputs = Array.from(
      host?.querySelectorAll('input[name^="batchOauthGroupName-"]') ?? [],
    ) as HTMLInputElement[];
    expect(groupInputs[0]?.value).toBe("prod");
    expect(groupInputs[4]?.value).toBe("prod");

    clickButton(/Add row/i);
    await flushAsync();

    const updatedGroupInputs = Array.from(
      host?.querySelectorAll('input[name^="batchOauthGroupName-"]') ?? [],
    ) as HTMLInputElement[];
    expect(updatedGroupInputs[5]?.value).toBe("prod");
  });

  it("clears a pending row session when metadata changes", async () => {
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ beginOauthLogin });
    render("/account-pool/upstream-accounts/new?mode=batchOauth");

    setInputValue('input[name^="batchOauthDisplayName-"]', "Row One");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    expect(beginOauthLogin).toHaveBeenCalledWith({
      displayName: "Row One",
      groupName: undefined,
      groupNote: undefined,
      note: undefined,
      tagIds: [],
      isMother: false,
    });
    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(false);

    clickButton(/Expand note/i);
    setInputValue('input[name^="batchOauthNote-"]', "Needs a new login");

    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(true);
    expect(host?.textContent).toContain(
      "Metadata changed. Generate a fresh OAuth URL for this row before completing login.",
    );
  });

  it("completes one row without leaving the batch page", async () => {
    const beginOauthLogin = vi
      .fn()
      .mockResolvedValueOnce({
        loginId: "login-1",
        status: "pending",
        authUrl: "https://auth.openai.com/authorize?login=1",
        redirectUri: "http://localhost:1455/oauth/callback",
        expiresAt: "2026-03-13T10:00:00.000Z",
        accountId: null,
        error: null,
      })
      .mockResolvedValueOnce({
        loginId: "login-2",
        status: "pending",
        authUrl: "https://auth.openai.com/authorize?login=2",
        redirectUri: "http://localhost:1455/oauth/callback",
        expiresAt: "2026-03-13T10:00:00.000Z",
        accountId: null,
        error: null,
      });
    const completeOauthLogin = vi
      .fn()
      .mockResolvedValue({ id: 41, displayName: "Row One" });
    mockUpstreamAccounts({ beginOauthLogin, completeOauthLogin });
    render("/account-pool/upstream-accounts/new?mode=batchOauth");

    clickButton(/Add row/i);
    expect(getBatchRows()).toHaveLength(6);

    const displayNames =
      host?.querySelectorAll('input[name^="batchOauthDisplayName-"]') ?? [];
    const inputSetter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )?.set;
    if (!inputSetter) throw new Error("missing native input setter");
    act(() => {
      inputSetter.call(displayNames[0], "Row One");
      displayNames[0]?.dispatchEvent(new Event("input", { bubbles: true }));
      displayNames[0]?.dispatchEvent(new Event("change", { bubbles: true }));
      inputSetter.call(displayNames[1], "Row Two");
      displayNames[1]?.dispatchEvent(new Event("input", { bubbles: true }));
      displayNames[1]?.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await flushAsync();

    let generateButtons = Array.from(
      host?.querySelectorAll("button") ?? [],
    ).filter((button) =>
      /Generate OAuth URL/.test(
        button.textContent ||
          button.getAttribute("aria-label") ||
          button.getAttribute("title") ||
          "",
      ),
    );
    act(() => {
      generateButtons[0]?.dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
    });
    await flushAsync();
    generateButtons = Array.from(host?.querySelectorAll("button") ?? []).filter(
      (button) =>
        /Generate OAuth URL|Regenerate OAuth URL/.test(
          button.textContent ||
            button.getAttribute("aria-label") ||
            button.getAttribute("title") ||
            "",
        ),
    );
    act(() => {
      generateButtons[1]?.dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
    });
    await flushAsync();

    const callbackInputs =
      host?.querySelectorAll('input[name^="batchOauthCallbackUrl-"]') ?? [];
    act(() => {
      inputSetter.call(
        callbackInputs[0],
        "http://localhost:1455/oauth/callback?code=row-one",
      );
      callbackInputs[0]?.dispatchEvent(new Event("input", { bubbles: true }));
      callbackInputs[0]?.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await flushAsync();

    const completeButtons = Array.from(
      host?.querySelectorAll("button") ?? [],
    ).filter((button) =>
      /Complete OAuth login/.test(
        button.textContent ||
          button.getAttribute("aria-label") ||
          button.getAttribute("title") ||
          "",
      ),
    );
    act(() => {
      completeButtons[0]?.dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
    });
    await flushAsync();

    expect(completeOauthLogin).toHaveBeenCalledWith("login-1", {
      callbackUrl: "http://localhost:1455/oauth/callback?code=row-one",
    });
    expect(host?.textContent).toContain(
      "Row One is ready. Continue with the remaining rows when you are done here.",
    );
    expect(getBatchRows()).toHaveLength(6);
    expect(navigateMock).not.toHaveBeenCalled();
  });

  it("blocks completing a batch row when another draft row reuses the same display name", async () => {
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ beginOauthLogin });
    render("/account-pool/upstream-accounts/new?mode=batchOauth");

    const displayNames =
      host?.querySelectorAll('input[name^="batchOauthDisplayName-"]') ?? [];
    const inputSetter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )?.set;
    if (!inputSetter) throw new Error("missing native input setter");
    act(() => {
      inputSetter.call(displayNames[0], "Shared Name");
      displayNames[0]?.dispatchEvent(new Event("input", { bubbles: true }));
      displayNames[0]?.dispatchEvent(new Event("change", { bubbles: true }));
      inputSetter.call(displayNames[1], " shared name ");
      displayNames[1]?.dispatchEvent(new Event("input", { bubbles: true }));
      displayNames[1]?.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await flushAsync();

    expect(host?.textContent).toContain("Display name must be unique.");
  });

  it("attaches a supported mailbox from the batch popover editor", async () => {
    const beginOauthMailboxSessionForAddress = vi.fn().mockResolvedValue({
      supported: true,
      sessionId: "mailbox-attached-row-1",
      emailAddress: "edited-batch@mail-tw.707079.xyz",
      expiresAt: "2026-04-13T10:00:00.000Z",
      source: "attached",
    });
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-batch-mailbox",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=batch-mailbox",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ beginOauthMailboxSessionForAddress, beginOauthLogin });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=batchOauth",
      state: {
        draft: {
          batchOauth: {
            rows: [
              {
                id: "row-1",
                displayName: "Batch Row",
                groupName: "prod",
                mailboxSession: {
                  supported: true,
                  sessionId: "mailbox-original-row-1",
                  emailAddress: "original-batch@mail-tw.707079.xyz",
                  expiresAt: "2026-04-13T10:00:00.000Z",
                  source: "generated",
                },
                mailboxInput: "original-batch@mail-tw.707079.xyz",
              },
            ],
          },
        },
      },
    });
    await flushAsync();

    const mailboxChip = findBodyButton(/Copy mailbox/i);
    expect(mailboxChip).toBeInstanceOf(HTMLButtonElement);
    act(() => {
      mailboxChip?.dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));
      mailboxChip?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    });
    await flushAsync();

    const editMailboxButton = document.body.querySelector('button[title="Edit mailbox"]');
    if (!(editMailboxButton instanceof HTMLButtonElement)) {
      throw new Error("missing edit mailbox button");
    }
    act(() => {
      editMailboxButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushAsync();
    setBodyInputValue('input[name="batchOauthMailboxEditor-row-1"]', "edited-batch@mail-tw.707079.xyz");
    clickBodyButton(/Submit mailbox/i);
    await flushAsync();

    expect(beginOauthMailboxSessionForAddress).toHaveBeenCalledWith("edited-batch@mail-tw.707079.xyz");
    expect(host?.textContent).toContain("edited-batch@mail-tw.707079.xyz");

    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    expect(beginOauthLogin).toHaveBeenCalledWith({
      displayName: "Batch Row",
      groupName: "prod",
      note: undefined,
      tagIds: [],
      groupNote: undefined,
      isMother: false,
      mailboxSessionId: "mailbox-attached-row-1",
      mailboxAddress: "edited-batch@mail-tw.707079.xyz",
    });
  });

  it("keeps batch oauth actions available when the edited mailbox is unsupported", async () => {
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-batch-unsupported",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=batch-unsupported",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    const beginOauthMailboxSessionForAddress = vi.fn().mockResolvedValue({
      supported: false,
      emailAddress: "unsupported@example.com",
      reason: "not_readable",
    });
    mockUpstreamAccounts({ beginOauthLogin, beginOauthMailboxSessionForAddress });
    render("/account-pool/upstream-accounts/new?mode=batchOauth");
    await flushAsync();

    setInputValue('input[name^="batchOauthDisplayName-"]', "Batch Unsupported");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    const mailboxChip = findBodyButton(/Edit mailbox/i);
    expect(mailboxChip).toBeInstanceOf(HTMLButtonElement);
    act(() => {
      mailboxChip?.dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));
      mailboxChip?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    });
    await flushAsync();

    const editMailboxButton = document.body.querySelector('button[title="Edit mailbox"]');
    if (!(editMailboxButton instanceof HTMLButtonElement)) {
      throw new Error("missing edit mailbox button");
    }
    act(() => {
      editMailboxButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushAsync();
    setBodyInputValue('input[name^="batchOauthMailboxEditor-"]', "unsupported@example.com");
    clickBodyButton(/Submit mailbox/i);
    await flushAsync();

    expect(beginOauthMailboxSessionForAddress).toHaveBeenCalledWith("unsupported@example.com");
    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(false);
    expect(host?.textContent).toContain(
      "This mailbox is not readable through the current MoeMail integration, so mailbox enhancements stay disabled.",
    );
  });

  it("validates the batch mailbox editor before attaching", async () => {
    const beginOauthMailboxSessionForAddress = vi.fn();
    mockUpstreamAccounts({ beginOauthMailboxSessionForAddress });
    render("/account-pool/upstream-accounts/new?mode=batchOauth");
    await flushAsync();

    const mailboxChip = findBodyButton(/Edit mailbox/i);
    expect(mailboxChip).toBeInstanceOf(HTMLButtonElement);
    act(() => {
      mailboxChip?.dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));
      mailboxChip?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    });
    await flushAsync();

    const editMailboxButton = document.body.querySelector('button[title="Edit mailbox"]');
    if (!(editMailboxButton instanceof HTMLButtonElement)) {
      throw new Error("missing edit mailbox button");
    }
    act(() => {
      editMailboxButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushAsync();

    setBodyInputValue('input[name^="batchOauthMailboxEditor-"]', "not-an-email");
    clickBodyButton(/Submit mailbox/i);
    await flushAsync();

    expect(beginOauthMailboxSessionForAddress).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("Enter a valid email address before attaching it.");
  });

  it("cancels batch mailbox editing without mutating the row mailbox value", async () => {
    mockUpstreamAccounts();
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=batchOauth",
      state: {
        draft: {
          batchOauth: {
            rows: [
              {
                id: "row-1",
                displayName: "Batch Row",
                mailboxSession: {
                  supported: true,
                  sessionId: "mailbox-original-row-1",
                  emailAddress: "original-batch@mail-tw.707079.xyz",
                  expiresAt: "2026-04-13T10:00:00.000Z",
                  source: "generated",
                },
                mailboxInput: "original-batch@mail-tw.707079.xyz",
              },
            ],
          },
        },
      },
    });
    await flushAsync();

    const mailboxChip = findBodyButton(/Copy mailbox/i);
    expect(mailboxChip).toBeInstanceOf(HTMLButtonElement);
    act(() => {
      mailboxChip?.dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));
      mailboxChip?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    });
    await flushAsync();

    const editMailboxButton = document.body.querySelector('button[title="Edit mailbox"]');
    if (!(editMailboxButton instanceof HTMLButtonElement)) {
      throw new Error("missing edit mailbox button");
    }
    act(() => {
      editMailboxButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushAsync();
    setBodyInputValue('input[name="batchOauthMailboxEditor-row-1"]', "edited-batch@mail-tw.707079.xyz");
    clickBodyButton(/Cancel mailbox edit/i);
    await flushAsync();

    expect(host?.textContent).toContain("original-batch@mail-tw.707079.xyz");
    expect(host?.textContent).not.toContain("edited-batch@mail-tw.707079.xyz");
  });

  it("invalidates an existing batch oauth URL after attaching a new supported mailbox", async () => {
    const beginOauthMailboxSessionForAddress = vi.fn().mockResolvedValue({
      supported: true,
      sessionId: "mailbox-attached-row-1",
      emailAddress: "edited-batch@mail-tw.707079.xyz",
      expiresAt: "2026-04-13T10:00:00.000Z",
      source: "attached",
    });
    mockUpstreamAccounts({ beginOauthMailboxSessionForAddress });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=batchOauth",
      state: {
        draft: {
          batchOauth: {
            rows: [
              {
                id: "row-1",
                displayName: "Batch Row",
                mailboxSession: {
                  supported: true,
                  sessionId: "mailbox-original-row-1",
                  emailAddress: "original-batch@mail-tw.707079.xyz",
                  expiresAt: "2026-04-13T10:00:00.000Z",
                  source: "generated",
                },
                mailboxInput: "original-batch@mail-tw.707079.xyz",
                session: {
                  loginId: "login-existing-row-1",
                  status: "pending",
                  authUrl: "https://auth.openai.com/authorize?login=existing-row-1",
                  redirectUri: "http://localhost:1455/oauth/callback",
                  expiresAt: "2026-03-13T10:00:00.000Z",
                  accountId: null,
                  error: null,
                },
                sessionHint: "OAuth URL ready",
              },
            ],
          },
        },
      },
    });
    await flushAsync();

    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(false);

    const mailboxChip = findBodyButton(/Copy mailbox/i);
    expect(mailboxChip).toBeInstanceOf(HTMLButtonElement);
    act(() => {
      mailboxChip?.dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));
      mailboxChip?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    });
    await flushAsync();

    const editMailboxButton = document.body.querySelector('button[title="Edit mailbox"]');
    if (!(editMailboxButton instanceof HTMLButtonElement)) {
      throw new Error("missing edit mailbox button");
    }
    act(() => {
      editMailboxButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushAsync();
    setBodyInputValue('input[name="batchOauthMailboxEditor-row-1"]', "edited-batch@mail-tw.707079.xyz");
    clickBodyButton(/Submit mailbox/i);
    await flushAsync();

    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(true);
    expect((host?.querySelector('input[name="batchOauthCallbackUrl-row-1"]') as HTMLInputElement | null)?.value).toBe("");
  });
});

describe("UpstreamAccountCreatePage display name validation", () => {
  it('does not resend an existing shared group note when generating oauth for a grouped row', async () => {
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: 'login-1',
      status: 'pending',
      authUrl: 'https://auth.openai.com/authorize?login=1',
      redirectUri: 'http://localhost:1455/oauth/callback',
      expiresAt: '2026-03-13T10:00:00.000Z',
      accountId: null,
      error: null,
    })
    mockUpstreamAccounts({ beginOauthLogin })
    render('/account-pool/upstream-accounts/new?mode=batchOauth')

    setComboboxValue('input[name="batchOauthDefaultGroupName"]', 'prod')
    await flushAsync()

    clickButton(/Generate OAuth URL/i)
    await flushAsync()

    expect(beginOauthLogin).toHaveBeenCalledWith({
      displayName: undefined,
      groupName: 'prod',
      tagIds: [],
      groupNote: undefined,
      note: undefined,
      isMother: false,
    })
  })

  it('passes a draft shared group note when generating oauth for a new grouped row', async () => {
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: 'login-1',
      status: 'pending',
      authUrl: 'https://auth.openai.com/authorize?login=1',
      redirectUri: 'http://localhost:1455/oauth/callback',
      expiresAt: '2026-03-13T10:00:00.000Z',
      accountId: null,
      error: null,
    })
    mockUpstreamAccounts({ beginOauthLogin })
    render('/account-pool/upstream-accounts/new?mode=batchOauth')

    setComboboxValue('input[name="batchOauthDefaultGroupName"]', 'new-team')
    await flushAsync()

    clickButton(/Edit group note/i)
    await flushAsync()
    const draftGroupNoteField =
      document.body.querySelector('textarea') ??
      (() => {
        throw new Error('missing group note textarea')
      })()
    if (!(draftGroupNoteField instanceof HTMLTextAreaElement)) {
      throw new Error('missing group note textarea')
    }
    setFieldValue(draftGroupNoteField, 'Draft shared group note')
    clickBodyButton(/Save changes/i)
    await flushAsync()

    clickButton(/Generate OAuth URL/i)
    await flushAsync()

    expect(beginOauthLogin).toHaveBeenCalledWith({
      displayName: undefined,
      groupName: 'new-team',
      tagIds: [],
      groupNote: 'Draft shared group note',
      note: undefined,
      isMother: false,
    })
  })

  it("blocks completing single oauth when the display name already exists", async () => {
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ beginOauthLogin });
    render();

    setInputValue('input[name="oauthDisplayName"]', " existing oauth ");
    await flushAsync();

    expect(host?.textContent).toContain("Display name must be unique.");
    expect(findButton(/Generate OAuth URL/i)?.disabled).toBe(true);
    expect(findButton(/Complete OAuth login/i)?.disabled).toBe(true);
    expect(beginOauthLogin).not.toHaveBeenCalled();
  });

  it("invalidates a pending single oauth session when metadata changes", async () => {
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ beginOauthLogin });
    render();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(false);

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth Renamed");
    await flushAsync();

    expect(host?.textContent).toContain("Generate a fresh OAuth URL");
    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(true);
    expect(findButton(/Complete OAuth login/i)?.disabled).toBe(true);
  });

  it('invalidates pending batch oauth sessions when a draft group note changes', async () => {
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: 'login-1',
      status: 'pending',
      authUrl: 'https://auth.openai.com/authorize?login=1',
      redirectUri: 'http://localhost:1455/oauth/callback',
      expiresAt: '2026-03-13T10:00:00.000Z',
      accountId: null,
      error: null,
    })
    mockUpstreamAccounts({ beginOauthLogin })
    render('/account-pool/upstream-accounts/new?mode=batchOauth')

    setComboboxValue('input[name="batchOauthDefaultGroupName"]', 'new-team')
    await flushAsync()

    clickButton(/Generate OAuth URL/i)
    await flushAsync()
    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(false)

    clickButton(/Edit group note/i)
    await flushAsync()
    const updatedGroupNoteField =
      document.body.querySelector('textarea') ??
      (() => {
        throw new Error('missing group note textarea')
      })()
    if (!(updatedGroupNoteField instanceof HTMLTextAreaElement)) {
      throw new Error('missing group note textarea')
    }
    setFieldValue(updatedGroupNoteField, 'Updated draft shared note')
    clickBodyButton(/Save changes/i)
    await flushAsync()

    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(true)
    expect(host?.textContent).toContain('Metadata changed. Generate a fresh OAuth URL for this row before completing login.')
  })

  it("blocks creating an API key account when the display name already exists", async () => {
    mockUpstreamAccounts();
    render();

    clickButton(/^API key$/i);
    setInputValue('input[name="apiKeyDisplayName"]', "Existing OAuth");
    await flushAsync();

    expect(host?.textContent).toContain("Display name must be unique.");
    expect(findButton(/Create API key account/i)?.disabled).toBe(true);
  });

  it("shows duplicate warnings on the create page after single oauth completes", async () => {
    const completeOauthLogin = vi.fn().mockResolvedValue({
      id: 41,
      displayName: "Fresh OAuth",
      duplicateInfo: {
        peerAccountIds: [5],
        reasons: ["sharedChatgptAccountId"],
      },
    });
    mockUpstreamAccounts({ completeOauthLogin });
    render();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();
    setInputValue(
      'textarea[name="oauthCallbackUrl"]',
      "http://localhost:1455/oauth/callback?code=test",
    );
    await flushAsync();
    clickButton(/Complete OAuth login/i);
    await flushAsync();

    expect(document.body.textContent).toContain(
      "Possible upstream duplicate",
    );
    expect(document.body.textContent).toContain(
      "Matched: shared ChatGPT account id. Related account ids: 5.",
    );
    expect(navigateMock).not.toHaveBeenCalled();
  });

  it("refreshes the single oauth session after a server-side completion failure", async () => {
    const getLoginSession = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "failed",
      authUrl: null,
      redirectUri: null,
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: "Display name must be unique.",
    });
    const completeOauthLogin = vi
      .fn()
      .mockRejectedValue(new Error("Display name must be unique."));
    mockUpstreamAccounts({ completeOauthLogin, getLoginSession });
    render();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();
    setInputValue(
      'textarea[name="oauthCallbackUrl"]',
      "http://localhost:1455/oauth/callback?code=test",
    );
    await flushAsync();
    clickButton(/Complete OAuth login/i);
    await flushAsync();

    expect(getLoginSession).toHaveBeenCalledWith("login-1");
    expect(document.body.textContent).toContain(
      "Display name must be unique.",
    );
    expect(findButton(/Complete OAuth login/i)?.disabled).toBe(true);
  });

  it("recovers from a lost completion response when the session is already completed", async () => {
    const getLoginSession = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "completed",
      authUrl: null,
      redirectUri: null,
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: 41,
      error: null,
    });
    const completeOauthLogin = vi
      .fn()
      .mockRejectedValue(new Error("network failed"));
    mockUpstreamAccounts({ completeOauthLogin, getLoginSession });
    render();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();
    setInputValue(
      'textarea[name="oauthCallbackUrl"]',
      "http://localhost:1455/oauth/callback?code=test",
    );
    await flushAsync();
    clickButton(/Complete OAuth login/i);
    await flushAsync();

    expect(getLoginSession).toHaveBeenCalledWith("login-1");
    expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenCalledWith(41);
    expect(navigateMock).toHaveBeenCalledWith(
      "/account-pool/upstream-accounts",
      {
        state: {
          selectedAccountId: 41,
          openDetail: true,
          duplicateWarning: null,
        },
      },
    );
    expect(document.body.textContent).not.toContain("network failed");
    expect(findButton(/Generate OAuth URL/i)?.disabled).toBe(true);
  });

  it("recovers a batch row when the callback already completed on the server", async () => {
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    const getLoginSession = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "completed",
      authUrl: null,
      redirectUri: null,
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: 41,
      error: null,
    });
    const completeOauthLogin = vi
      .fn()
      .mockRejectedValue(new Error("network failed"));
    mockUpstreamAccounts({ beginOauthLogin, completeOauthLogin, getLoginSession });
    render("/account-pool/upstream-accounts/new?mode=batchOauth");

    setInputValue('input[name^="batchOauthDisplayName-"]', "Row One");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();
    setInputValue(
      'input[name^="batchOauthCallbackUrl-"]',
      "http://localhost:1455/oauth/callback?code=row-one",
    );
    await flushAsync();
    clickButton(/Complete OAuth login/i);
    await flushAsync();

    expect(getLoginSession).toHaveBeenCalledWith("login-1");
    expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenCalledWith(41);
    expect(document.body.textContent).toContain(
      "Row One is ready. Continue with the remaining rows when you are done here.",
    );
    expect(document.body.textContent).not.toContain("network failed");
  });

  it("keeps a recovered batch row in caution state when final detail fetch fails", async () => {
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    const getLoginSession = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "completed",
      authUrl: null,
      redirectUri: null,
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: 41,
      error: null,
    });
    apiMocks.fetchUpstreamAccountDetail.mockRejectedValueOnce(
      new Error("detail fetch failed"),
    );
    const completeOauthLogin = vi
      .fn()
      .mockRejectedValue(new Error("network failed"));
    mockUpstreamAccounts({ beginOauthLogin, completeOauthLogin, getLoginSession });
    render("/account-pool/upstream-accounts/new?mode=batchOauth");

    setInputValue('input[name^="batchOauthDisplayName-"]', "Row One");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();
    setInputValue(
      'input[name^="batchOauthCallbackUrl-"]',
      "http://localhost:1455/oauth/callback?code=row-one",
    );
    await flushAsync();
    clickButton(/Complete OAuth login/i);
    await flushAsync();

    expect(getLoginSession).toHaveBeenCalledWith("login-1");
    expect(apiMocks.fetchUpstreamAccountDetail).toHaveBeenCalledWith(41);
    expect(document.body.textContent).toContain(
      "OAuth completed on the server. Refresh the roster to load the final account details.",
    );
    expect(document.body.textContent).not.toContain(
      "Row One is ready. Continue with the remaining rows when you are done here.",
    );
    expect(document.body.textContent).toContain("Needs refresh");
    const generateButtons = Array.from(
      host?.querySelectorAll("button") ?? [],
    ).filter((button) =>
      /Generate OAuth URL|Regenerate OAuth URL/.test(
        button.textContent ||
          button.getAttribute("aria-label") ||
          button.getAttribute("title") ||
          "",
      ),
    );
    expect(generateButtons[0]).toBeInstanceOf(HTMLButtonElement);
    expect((generateButtons[0] as HTMLButtonElement).disabled).toBe(true);
  });

  it("shows duplicate warnings inline after completing a batch oauth row", async () => {
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    const completeOauthLogin = vi.fn().mockResolvedValue({
      id: 41,
      displayName: "Row One",
      duplicateInfo: {
        peerAccountIds: [5],
        reasons: ["sharedChatgptUserId"],
      },
    });
    mockUpstreamAccounts({ beginOauthLogin, completeOauthLogin });
    render("/account-pool/upstream-accounts/new?mode=batchOauth");

    setInputValue('input[name^="batchOauthDisplayName-"]', "Row One");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();
    setInputValue(
      'input[name^="batchOauthCallbackUrl-"]',
      "http://localhost:1455/oauth/callback?code=row-one",
    );
    await flushAsync();
    clickButton(/Complete OAuth login/i);
    await flushAsync();

    expect(document.body.textContent).toContain(
      "Possible upstream duplicate",
    );
    expect(document.body.textContent).toContain(
      "Matched: shared ChatGPT user id. Related account ids: 5.",
    );
  });
});

describe("UpstreamAccountCreatePage oauth mailbox", () => {
  it("fills the display name with the mailbox when it is blank", async () => {
    const beginOauthMailboxSession = vi.fn().mockResolvedValue({
      supported: true,
      sessionId: "mailbox-1",
      emailAddress: "temp-user@example.com",
      expiresAt: "2026-03-13T10:00:00.000Z",
      source: "generated",
    });
    mockUpstreamAccounts({ beginOauthMailboxSession });
    render("/account-pool/upstream-accounts/new?mode=oauth");

    const displayNameInput = host?.querySelector(
      'input[name="oauthDisplayName"]',
    ) as HTMLInputElement | null;
    expect(displayNameInput).toBeInstanceOf(HTMLInputElement);

    clickButton(/Generate/i);
    await flushAsync();

    expect(beginOauthMailboxSession).toHaveBeenCalledTimes(1);
    expect(displayNameInput?.value).toBe("temp-user@example.com");
    expect(host?.textContent).toContain("temp-user@example.com");
    expect(
      Array.from(host?.querySelectorAll("button") ?? []).some(
        (candidate) =>
          candidate instanceof HTMLButtonElement &&
          /Copy mailbox/i.test(
            candidate.getAttribute("aria-label") || candidate.textContent || "",
          ),
      ),
    ).toBe(true);
  });

  it("keeps the display name when it already has visible characters", async () => {
    const beginOauthMailboxSession = vi.fn().mockResolvedValue({
      supported: true,
      sessionId: "mailbox-2",
      emailAddress: "temp-user-2@example.com",
      expiresAt: "2026-03-13T10:00:00.000Z",
      source: "generated",
    });
    mockUpstreamAccounts({ beginOauthMailboxSession });
    render("/account-pool/upstream-accounts/new?mode=oauth");

    const displayNameInput = setInputValue('input[name="oauthDisplayName"]', "Manual Alias");

    clickButton(/Generate/i);
    await flushAsync();

    expect(beginOauthMailboxSession).toHaveBeenCalledTimes(1);
    expect(displayNameInput.value).toBe("Manual Alias");
    expect(host?.textContent).toContain("temp-user-2@example.com");
  });

  it("invalidates a mailbox-bound oauth session when the mailbox draft changes", async () => {
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ beginOauthLogin });
    render("/account-pool/upstream-accounts/new?mode=oauth");

    clickButton(/Generate/i);
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    setInputValue('input[name="oauthMailboxInput"]', "new-target@example.com");
    await flushAsync();

    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(true);
    expect(host?.textContent).toContain(
      "Generate a fresh OAuth URL before completing login.",
    );
  });

  it("keeps the pending oauth url when an unsupported mailbox attach falls back", async () => {
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    const beginOauthMailboxSessionForAddress = vi.fn().mockResolvedValue({
      supported: false,
      emailAddress: "manual-existing@example.com",
      reason: "not_readable",
    });
    mockUpstreamAccounts({ beginOauthLogin, beginOauthMailboxSessionForAddress });
    render("/account-pool/upstream-accounts/new?mode=oauth");

    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    setInputValue('input[name="oauthMailboxInput"]', "manual-existing@example.com");
    await flushAsync();
    clickButton(/Use address/i);
    await flushAsync();

    expect(beginOauthMailboxSessionForAddress).toHaveBeenCalledWith("manual-existing@example.com");
    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(false);
    expect(host?.textContent).not.toContain(
      "Generate a fresh OAuth URL for this row before completing login.",
    );
  });

  it("attaches a supported manual mailbox address without blocking oauth actions", async () => {
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    const beginOauthMailboxSessionForAddress = vi.fn().mockResolvedValue({
      supported: true,
      sessionId: "mailbox-attached-9",
      emailAddress: "manual-existing@mail-tw.707079.xyz",
      expiresAt: "2026-03-13T10:00:00.000Z",
      source: "attached",
    });
    mockUpstreamAccounts({ beginOauthLogin, beginOauthMailboxSessionForAddress });
    render("/account-pool/upstream-accounts/new?mode=oauth");

    setInputValue('input[name="oauthMailboxInput"]', "manual-existing@mail-tw.707079.xyz");
    await flushAsync();
    clickButton(/Use address/i);
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    expect(beginOauthMailboxSessionForAddress).toHaveBeenCalledWith("manual-existing@mail-tw.707079.xyz");
    expect(beginOauthLogin).toHaveBeenCalledWith({
      displayName: "manual-existing@mail-tw.707079.xyz",
      groupName: undefined,
      note: undefined,
      groupNote: undefined,
      accountId: undefined,
      tagIds: [],
      isMother: false,
      mailboxSessionId: "mailbox-attached-9",
      mailboxAddress: "manual-existing@mail-tw.707079.xyz",
    });
    expect(host?.textContent).toContain("manual-existing@mail-tw.707079.xyz");
    expect(host?.textContent).toContain("Attached mailbox");
    expect(findButton(/Generate OAuth URL/i)?.disabled).toBe(false);
  });

  it("stops using a supported mailbox session after the input diverges", async () => {
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ beginOauthLogin });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=oauth",
      state: {
        draft: {
          oauth: {
            displayName: "Mailbox Drift",
            mailboxSession: {
              supported: true,
              sessionId: "mailbox-attached-9",
              emailAddress: "manual-existing@mail-tw.707079.xyz",
              expiresAt: "2026-03-13T10:00:00.000Z",
              source: "attached",
            },
            mailboxInput: "manual-existing@mail-tw.707079.xyz",
            mailboxStatus: {
              sessionId: "mailbox-attached-9",
              emailAddress: "manual-existing@mail-tw.707079.xyz",
              expiresAt: "2026-03-13T10:00:00.000Z",
              latestCode: {
                value: "123456",
                updatedAt: "2026-03-13T09:59:00.000Z",
              },
              invite: {
                subject: "Invite",
                copyValue: "invite-link",
              },
              invited: true,
              error: null,
            },
          },
        },
      },
    });

    await flushAsync();
    setInputValue('input[name="oauthMailboxInput"]', "different@example.com");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    expect(beginOauthLogin).toHaveBeenCalledWith({
      displayName: "Mailbox Drift",
      groupName: undefined,
      note: undefined,
      groupNote: undefined,
      accountId: undefined,
      tagIds: [],
      isMother: false,
      mailboxSessionId: undefined,
      mailboxAddress: undefined,
    });
    expect(findButton(/Copy code/i)?.disabled).toBe(true);
    expect(host?.textContent).not.toContain("Attached mailbox");

    setInputValue('input[name="oauthMailboxInput"]', "manual-existing@mail-tw.707079.xyz");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    expect(beginOauthLogin).toHaveBeenLastCalledWith({
      displayName: "Mailbox Drift",
      groupName: undefined,
      note: undefined,
      groupNote: undefined,
      accountId: undefined,
      tagIds: [],
      isMother: false,
      mailboxSessionId: undefined,
      mailboxAddress: undefined,
    });
    expect(host?.textContent).not.toContain("Attached mailbox");
  });

  it("does not delete a generated mailbox remotely when the draft input changes", async () => {
    const removeOauthMailboxSession = vi.fn().mockResolvedValue(undefined);
    mockUpstreamAccounts({ removeOauthMailboxSession });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=oauth",
      state: {
        draft: {
          oauth: {
            displayName: "Generated Mailbox",
            mailboxSession: {
              supported: true,
              sessionId: "mailbox-generated-1",
              emailAddress: "generated@mail-tw.707079.xyz",
              expiresAt: "2026-03-13T10:00:00.000Z",
              source: "generated",
            },
            mailboxInput: "generated@mail-tw.707079.xyz",
          },
        },
      },
    });

    await flushAsync();
    setInputValue('input[name="oauthMailboxInput"]', "manual-existing@example.com");
    await flushAsync();

    expect(removeOauthMailboxSession).not.toHaveBeenCalled();
    expect(host?.textContent).not.toContain("Generated mailbox");
  });

  it("keeps a supported mailbox session when the input only changes casing", async () => {
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    const removeOauthMailboxSession = vi.fn().mockResolvedValue(undefined);
    mockUpstreamAccounts({ beginOauthLogin, removeOauthMailboxSession });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=oauth",
      state: {
        draft: {
          oauth: {
            displayName: "Mailbox Case",
            mailboxSession: {
              supported: true,
              sessionId: "mailbox-attached-case",
              emailAddress: "manual-existing@mail-tw.707079.xyz",
              expiresAt: "2026-03-13T10:00:00.000Z",
              source: "attached",
            },
            mailboxInput: "manual-existing@mail-tw.707079.xyz",
          },
        },
      },
    });

    await flushAsync();
    setInputValue('input[name="oauthMailboxInput"]', "MANUAL-EXISTING@MAIL-TW.707079.XYZ");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    expect(beginOauthLogin).toHaveBeenCalledWith({
      displayName: "Mailbox Case",
      groupName: undefined,
      note: undefined,
      groupNote: undefined,
      accountId: undefined,
      tagIds: [],
      isMother: false,
      mailboxSessionId: "mailbox-attached-case",
      mailboxAddress: "manual-existing@mail-tw.707079.xyz",
    });
    expect(removeOauthMailboxSession).not.toHaveBeenCalled();
    expect(host?.textContent).toContain("Attached mailbox");
  });

  it("keeps oauth flow available when a manual mailbox is unsupported", async () => {
    const beginOauthMailboxSessionForAddress = vi.fn().mockResolvedValue({
      supported: false,
      emailAddress: "manual-existing@example.com",
      reason: "not_readable",
    });
    mockUpstreamAccounts({ beginOauthMailboxSessionForAddress });
    render("/account-pool/upstream-accounts/new?mode=oauth");

    setInputValue('input[name="oauthMailboxInput"]', "manual-existing@example.com");
    await flushAsync();
    clickButton(/Use address/i);
    await flushAsync();

    expect(beginOauthMailboxSessionForAddress).toHaveBeenCalledWith("manual-existing@example.com");
    expect(host?.textContent).toContain(
      "This mailbox is not readable through the current MoeMail integration, so mailbox enhancements stay disabled.",
    );
    expect(findButton(/Generate OAuth URL/i)?.disabled).toBe(false);
    expect(findButton(/Copy code/i)?.disabled).toBe(true);
  });

  it("shows an explicit expired mailbox warning for single oauth", async () => {
    const expiredAt = new Date(Date.now() - 60_000).toISOString()
    mockUpstreamAccounts()
    render({
      pathname: "/account-pool/upstream-accounts/new",
      state: {
        draft: {
          oauth: {
            displayName: "Expired Mailbox",
            mailboxSession: {
              supported: true,
              sessionId: "mailbox-expired",
              emailAddress: "expired@example.com",
              expiresAt: expiredAt,
              source: "generated",
            },
            mailboxInput: "expired@example.com",
          },
        },
      },
    });

    await flushAsync();

    expect(host?.textContent).toContain(
      "This temp mailbox has expired. Generate a fresh mailbox before waiting for new mail.",
    );
  });

  it("shows mailbox refresh failures instead of silently looking empty", async () => {
    const expiresAt = new Date(Date.now() + 60 * 60_000).toISOString()
    mockUpstreamAccounts({
      getOauthMailboxStatuses: vi.fn().mockRejectedValue(new Error("boom")),
    });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      state: {
        draft: {
          oauth: {
            displayName: "Refresh Failure Mailbox",
            mailboxSession: {
              supported: true,
              sessionId: "mailbox-refresh-failure",
              emailAddress: "failed@example.com",
              expiresAt,
              source: "generated",
            },
            mailboxInput: "failed@example.com",
          },
        },
      },
    });

    await flushAsync();
    await flushTimers();
    await flushAsync();

    expect(host?.textContent).toContain(
      "Mailbox refresh failed. We could not confirm the latest code or invite state.",
    );
  });
});

describe("UpstreamAccountCreatePage api key", () => {
  it("submits upstreamBaseUrl for API key accounts", async () => {
    const createApiKeyAccount = vi.fn().mockResolvedValue({
      id: 42,
      kind: "api_key_codex",
      provider: "codex",
      displayName: "Gateway Key",
      groupName: null,
      isMother: false,
      status: "active",
      enabled: true,
      history: [],
    });
    mockUpstreamAccounts({ createApiKeyAccount });
    render("/account-pool/upstream-accounts/new?mode=apiKey");

    setInputValue('input[name="apiKeyDisplayName"]', "Gateway Key");
    setInputValue('input[name="apiKeyValue"]', "sk-gateway");
    setInputValue(
      'input[name="apiKeyUpstreamBaseUrl"]',
      "https://proxy.example.com/gateway",
    );

    clickButton(/Create API Key account/i);
    await flushAsync();

    expect(createApiKeyAccount).toHaveBeenCalledWith(
      expect.objectContaining({
        displayName: "Gateway Key",
        apiKey: "sk-gateway",
        upstreamBaseUrl: "https://proxy.example.com/gateway",
      }),
    );
  });

  it("blocks api key creation when upstreamBaseUrl is not a valid absolute URL", () => {
    const createApiKeyAccount = vi.fn().mockResolvedValue({
      id: 42,
      kind: "api_key_codex",
      provider: "codex",
      displayName: "Gateway Key",
      groupName: null,
      isMother: false,
      status: "active",
      enabled: true,
      history: [],
    });
    mockUpstreamAccounts({ createApiKeyAccount });
    render("/account-pool/upstream-accounts/new?mode=apiKey");

    setInputValue('input[name="apiKeyDisplayName"]', "Gateway Key");
    setInputValue('input[name="apiKeyValue"]', "sk-gateway");
    setInputValue('input[name="apiKeyUpstreamBaseUrl"]', "proxy.example.com/gateway");

    expect(document.body.textContent).toContain("Use an absolute http(s) URL");
    expect(findButton(/Create API Key account/i)?.disabled).toBe(true);
    expect(createApiKeyAccount).not.toHaveBeenCalled();
  });
});
