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
import type {
  ImportedOauthValidationResponse,
  ImportedOauthValidationRow,
  LoginSessionStatusResponse,
} from "../../lib/api";
import UpstreamAccountCreatePage from "./UpstreamAccountCreate";

const navigateMock = vi.hoisted(() => vi.fn());
const hookMocks = vi.hoisted(() => ({
  useUpstreamAccounts: vi.fn(),
  usePoolTags: vi.fn(),
}));
const upstreamAccountsEventMocks = vi.hoisted(() => ({
  emitUpstreamAccountsChanged: vi.fn(),
}));
const apiMocks = vi.hoisted(() => ({
  fetchUpstreamAccountDetail: vi.fn(),
  createImportedOauthValidationJobEventSource: vi.fn(),
  updateOauthLoginSessionKeepalive: vi.fn(),
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

vi.mock("../../hooks/usePoolTags", () => ({
  usePoolTags: hookMocks.usePoolTags,
}));

vi.mock("../../lib/upstreamAccountsEvents", () => ({
  UPSTREAM_ACCOUNTS_CHANGED_EVENT: "upstream-accounts:changed",
  emitUpstreamAccountsChanged:
    upstreamAccountsEventMocks.emitUpstreamAccountsChanged,
}));

vi.mock("../../lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("../../lib/api")>("../../lib/api");
  return {
    ...actual,
    fetchUpstreamAccountDetail: apiMocks.fetchUpstreamAccountDetail,
    createImportedOauthValidationJobEventSource:
      apiMocks.createImportedOauthValidationJobEventSource,
    updateOauthLoginSessionKeepalive: apiMocks.updateOauthLoginSessionKeepalive,
  };
});

class MockValidationEventSource implements EventTarget {
  private listeners = new Map<string, Set<EventListener>>();
  readyState = 1;

  addEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
  ) {
    if (!listener) return;
    const handler =
      typeof listener === "function"
        ? listener
        : ((event: Event) => listener.handleEvent(event)) as EventListener;
    const current = this.listeners.get(type) ?? new Set<EventListener>();
    current.add(handler);
    this.listeners.set(type, current);
  }

  removeEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
  ) {
    if (!listener) return;
    const current = this.listeners.get(type);
    if (!current) return;
    const handler =
      typeof listener === "function"
        ? listener
        : ((event: Event) => listener.handleEvent(event)) as EventListener;
    current.delete(handler);
    if (current.size === 0) {
      this.listeners.delete(type);
    }
  }

  dispatchEvent(event: Event): boolean {
    const current = Array.from(this.listeners.get(event.type) ?? []);
    current.forEach((listener) => listener(event));
    return true;
  }

  close() {
    this.readyState = 2;
    this.listeners.clear();
  }

  emit(type: string, payload: unknown) {
    if (this.readyState === 2) return;
    this.dispatchEvent(
      new MessageEvent(type, {
        data: JSON.stringify(payload),
      }),
    );
  }
}

function buildImportedOauthValidationCounts(
  rows: Array<{ status: string }>,
) {
  const counts = {
    pending: 0,
    duplicateInInput: 0,
    ok: 0,
    okExhausted: 0,
    invalid: 0,
    error: 0,
    checked: 0,
  };
  rows.forEach((row) => {
    switch (row.status) {
      case "pending":
        counts.pending += 1;
        break;
      case "duplicate_in_input":
        counts.duplicateInInput += 1;
        break;
      case "ok":
        counts.ok += 1;
        break;
      case "ok_exhausted":
        counts.okExhausted += 1;
        break;
      case "invalid":
        counts.invalid += 1;
        break;
      case "error":
      default:
        counts.error += 1;
        break;
    }
  });
  counts.checked =
    counts.duplicateInInput +
    counts.ok +
    counts.okExhausted +
    counts.invalid +
    counts.error;
  return counts;
}

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
  upstreamAccountsEventMocks.emitUpstreamAccountsChanged.mockReset();
  apiMocks.createImportedOauthValidationJobEventSource.mockReset();
  apiMocks.updateOauthLoginSessionKeepalive.mockReset();
  apiMocks.updateOauthLoginSessionKeepalive.mockResolvedValue(undefined);
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
  apiMocks.createImportedOauthValidationJobEventSource.mockReset();
  apiMocks.updateOauthLoginSessionKeepalive.mockReset();
  vi.useRealTimers();
  vi.clearAllMocks();
});

function render(
  initialEntry: RenderEntry = "/account-pool/upstream-accounts/new",
) {
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

async function flushSessionSyncDebounce() {
  await act(async () => {
    vi.advanceTimersByTime(300);
    await Promise.resolve();
    await Promise.resolve();
  });
}

async function flushSessionSyncRetry() {
  await act(async () => {
    vi.advanceTimersByTime(1_100);
    await Promise.resolve();
    await Promise.resolve();
  });
}

async function setFileInputFiles(input: HTMLInputElement, files: File[]) {
  Object.defineProperty(input, "files", {
    configurable: true,
    value: files,
  });
  await act(async () => {
    input.dispatchEvent(new Event("change", { bubbles: true }));
    await Promise.resolve();
    await Promise.resolve();
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

function pageTextContent() {
  return document.body.textContent ?? "";
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
  const buildTagSummary = (tagId: number, name = `tag-${tagId}`) => ({
    id: tagId,
    name,
    routingRule: {
      guardEnabled: false,
      allowCutOut: true,
      allowCutIn: true,
    },
  });
  const buildSavedAccountDetail = (
    overrides: Record<string, unknown> = {},
  ) => ({
    id: 41,
    kind: "oauth_codex",
    provider: "codex",
    displayName: "Row One",
    groupName: "prod",
    isMother: false,
    status: "active",
    enabled: true,
    duplicateInfo: null,
    history: [],
    note: null,
    tags: [],
    effectiveRoutingRule: {
      guardEnabled: false,
      allowCutOut: true,
      allowCutIn: true,
      sourceTagIds: [],
      sourceTagNames: [],
      guardRules: [],
    },
    ...overrides,
  });
  hookMocks.usePoolTags.mockReturnValue({
    items: [
      {
        ...buildTagSummary(1, "vip"),
        accountCount: 1,
        groupCount: 1,
        updatedAt: "2026-03-13T10:00:00.000Z",
      },
      {
        ...buildTagSummary(2, "burst-safe"),
        accountCount: 0,
        groupCount: 0,
        updatedAt: "2026-03-13T10:00:00.000Z",
      },
    ],
    writesEnabled: true,
    isLoading: false,
    error: null,
    query: {},
    refresh: vi.fn(),
    updateQuery: vi.fn(),
    createTag: vi.fn(),
    updateTag: vi.fn(),
    deleteTag: vi.fn(),
  });
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
    updateOauthLogin: vi.fn().mockResolvedValue({
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
    runImportedOauthValidation: vi.fn().mockResolvedValue({
      inputFiles: 0,
      uniqueInInput: 0,
      duplicateInInput: 0,
      rows: [],
    }),
    startImportedOauthValidationJob: vi.fn().mockResolvedValue({
      jobId: "job-1",
      snapshot: {
        inputFiles: 0,
        uniqueInInput: 0,
        duplicateInInput: 0,
        rows: [],
      },
    }),
    stopImportedOauthValidationJob: vi.fn().mockResolvedValue(undefined),
    importOauthAccounts: vi.fn().mockResolvedValue({
      summary: {
        inputFiles: 0,
        selectedFiles: 0,
        created: 0,
        updatedExisting: 0,
        failed: 0,
      },
      results: [],
    }),
    saveGroupNote: vi
      .fn()
      .mockResolvedValue({ groupName: "prod", note: "Saved note" }),
    saveAccount: vi.fn().mockImplementation(
      async (accountId: number, payload: Record<string, unknown>) =>
        buildSavedAccountDetail({
          id: accountId,
          displayName:
            typeof payload.displayName === "string"
              ? payload.displayName
              : "Row One",
          groupName:
            typeof payload.groupName === "string" ? payload.groupName : "prod",
          note: typeof payload.note === "string" ? payload.note : null,
          isMother: payload.isMother === true,
          tags: Array.isArray(payload.tagIds)
            ? payload.tagIds.map((tagId) => buildTagSummary(Number(tagId)))
            : [],
        }),
    ),
    ...overrides,
  });
}

function blurField(selector: string) {
  const input = host?.querySelector(selector);
  if (
    !(input instanceof HTMLInputElement || input instanceof HTMLTextAreaElement)
  ) {
    throw new Error(`missing input for blur: ${selector}`);
  }
  act(() => {
    input.dispatchEvent(new FocusEvent("blur", { bubbles: true }));
    input.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
  });
  return input;
}

function buildCompletedBatchOauthRow(
  overrides: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    id: "row-1",
    displayName: "Row One",
    groupName: "prod",
    isMother: false,
    note: "Seed note",
    noteExpanded: true,
    callbackUrl: "http://localhost:1455/oauth/callback?code=row-one",
    mailboxSession: {
      supported: true,
      sessionId: "mailbox-row-1",
      emailAddress: "row-one@mail-tw.707079.xyz",
      expiresAt: "2026-04-13T10:00:00.000Z",
      source: "generated",
    },
    mailboxInput: "row-one@mail-tw.707079.xyz",
    session: {
      loginId: "login-1",
      status: "completed",
      authUrl: null,
      redirectUri: null,
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: 41,
      error: null,
    },
    metadataPersisted: {
      displayName: "Row One",
      groupName: "prod",
      note: "Seed note",
      isMother: false,
      tagIds: [],
    },
    ...overrides,
  };
}

describe("UpstreamAccountCreatePage batch oauth", () => {
  it("does not show background detail errors as the create-page banner", () => {
    mockUpstreamAccounts({
      detailError: "Background detail failed",
      error: "Background detail failed",
    });
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
  }, 10_000);

  it("keeps a pending row session while syncing metadata changes", async () => {
    vi.useFakeTimers();
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    const updateOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ beginOauthLogin, updateOauthLogin });
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
    await flushSessionSyncDebounce();

    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(false);
    expect(host?.textContent).not.toContain("Generate a fresh OAuth URL");
    expect(updateOauthLogin).toHaveBeenCalledWith("login-1", {
      displayName: "Row One",
      groupName: "",
      note: "Needs a new login",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
    expect(updateOauthLogin.mock.lastCall?.[1]).not.toHaveProperty("groupNote");
  }, 10_000);

  it("keeps completed oauth controls locked while saving completed-row metadata", async () => {
    const saveAccount = vi.fn().mockImplementation(
      async (accountId: number, payload: Record<string, unknown>) => ({
        id: accountId,
        kind: "oauth_codex",
        provider: "codex",
        displayName:
          typeof payload.displayName === "string"
            ? payload.displayName
            : "Row One",
        groupName:
          typeof payload.groupName === "string" ? payload.groupName : "prod",
        isMother: payload.isMother === true,
        status: "active",
        enabled: true,
        duplicateInfo: null,
        history: [],
        note: typeof payload.note === "string" ? payload.note : null,
        tags: Array.isArray(payload.tagIds)
          ? payload.tagIds.map((tagId) => ({
              id: Number(tagId),
              name: `tag-${tagId}`,
              routingRule: {
                guardEnabled: false,
                allowCutOut: true,
                allowCutIn: true,
              },
            }))
          : [],
        effectiveRoutingRule: {
          guardEnabled: false,
          allowCutOut: true,
          allowCutIn: true,
          sourceTagIds: [],
          sourceTagNames: [],
          guardRules: [],
        },
      }),
    );
    mockUpstreamAccounts({ saveAccount });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=batchOauth",
      state: {
        draft: {
          batchOauth: {
            defaultGroupName: "prod",
            tagIds: [1],
            rows: [
              buildCompletedBatchOauthRow({
                needsRefresh: true,
                metadataPersisted: {
                  displayName: "Row One",
                  groupName: "prod",
                  note: "Seed note",
                  isMother: false,
                  tagIds: [1],
                },
              }),
            ],
          },
        },
      },
    });
    await flushAsync();

    const displayNameInput = host?.querySelector(
      'input[name="batchOauthDisplayName-row-1"]',
    );
    const noteInput = host?.querySelector('input[name="batchOauthNote-row-1"]');
    const callbackInput = host?.querySelector(
      'input[name="batchOauthCallbackUrl-row-1"]',
    );
    expect(displayNameInput).toBeInstanceOf(HTMLInputElement);
    expect(noteInput).toBeInstanceOf(HTMLInputElement);
    expect(callbackInput).toBeInstanceOf(HTMLInputElement);
    expect((displayNameInput as HTMLInputElement).disabled).toBe(false);
    expect((noteInput as HTMLInputElement).disabled).toBe(false);
    expect((callbackInput as HTMLInputElement).disabled).toBe(true);
    expect(findButton(/Generate OAuth URL/i)?.disabled).toBe(true);
    expect(findButton(/Complete OAuth login/i)?.disabled).toBe(true);
    expect(findButton(/Remove row/i)?.disabled).toBe(true);
    expect(pageTextContent()).toContain("Needs refresh");

    setInputValue('input[name="batchOauthDisplayName-row-1"]', "Row One Renamed");
    blurField('input[name="batchOauthDisplayName-row-1"]');
    await flushAsync();

    setComboboxValue('input[name="batchOauthGroupName-row-1"]', "ops");
    await flushAsync();

    setInputValue('input[name="batchOauthNote-row-1"]', "Updated note");
    blurField('input[name="batchOauthNote-row-1"]');
    await flushAsync();

    clickButton(/Toggle mother account/i);
    await flushAsync();

    expect(saveAccount).toHaveBeenCalledTimes(4);
    const displayNamePayload = saveAccount.mock.calls[0]?.[1] as Record<
      string,
      unknown
    >;
    expect(displayNamePayload.displayName).toBe("Row One Renamed");
    expect(displayNamePayload.groupName).toBe("prod");
    expect(displayNamePayload.note).toBe("Seed note");
    expect(displayNamePayload.isMother).toBe(false);
    expect(displayNamePayload.tagIds).toEqual([1]);
    expect("email" in displayNamePayload).toBe(false);
    expect("mailboxAddress" in displayNamePayload).toBe(false);
    expect("callbackUrl" in displayNamePayload).toBe(false);

    const groupPayload = saveAccount.mock.calls[1]?.[1] as Record<
      string,
      unknown
    >;
    expect(groupPayload.groupName).toBe("ops");

    const notePayload = saveAccount.mock.calls[2]?.[1] as Record<
      string,
      unknown
    >;
    expect(notePayload.note).toBe("Updated note");

    const motherPayload = saveAccount.mock.calls[3]?.[1] as Record<
      string,
      unknown
    >;
    expect(motherPayload.isMother).toBe(true);
    expect(pageTextContent()).not.toContain("Needs refresh");
  }, 10_000);

  it("propagates the header default group to completed rows that still inherit it", async () => {
    const saveAccount = vi.fn().mockImplementation(
      async (accountId: number, payload: Record<string, unknown>) => ({
        id: accountId,
        kind: "oauth_codex",
        provider: "codex",
        displayName: accountId === 41 ? "Inherited Row" : "Custom Row",
        groupName:
          typeof payload.groupName === "string" ? payload.groupName : "alpha",
        isMother: false,
        status: "active",
        enabled: true,
        duplicateInfo: null,
        history: [],
        note: null,
        tags: [],
        effectiveRoutingRule: {
          guardEnabled: false,
          allowCutOut: true,
          allowCutIn: true,
          sourceTagIds: [],
          sourceTagNames: [],
          guardRules: [],
        },
      }),
    );
    mockUpstreamAccounts({ saveAccount });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=batchOauth",
      state: {
        draft: {
          batchOauth: {
            defaultGroupName: "alpha",
            rows: [
              buildCompletedBatchOauthRow({
                id: "row-1",
                displayName: "Inherited Row",
                groupName: "alpha",
                session: {
                  loginId: "login-1",
                  status: "completed",
                  authUrl: null,
                  redirectUri: null,
                  expiresAt: "2026-03-13T10:00:00.000Z",
                  accountId: 41,
                  error: null,
                },
                metadataPersisted: {
                  displayName: "Inherited Row",
                  groupName: "alpha",
                  note: "Seed note",
                  isMother: false,
                  tagIds: [],
                },
              }),
              buildCompletedBatchOauthRow({
                id: "row-2",
                displayName: "Custom Row",
                groupName: "custom",
                session: {
                  loginId: "login-2",
                  status: "completed",
                  authUrl: null,
                  redirectUri: null,
                  expiresAt: "2026-03-13T10:00:00.000Z",
                  accountId: 42,
                  error: null,
                },
                metadataPersisted: {
                  displayName: "Custom Row",
                  groupName: "custom",
                  note: "Seed note",
                  isMother: false,
                  tagIds: [],
                },
              }),
            ],
          },
        },
      },
    });
    await flushAsync();

    setComboboxValue('input[name="batchOauthDefaultGroupName"]', "beta");
    await flushAsync();

    const groupInputs = Array.from(
      host?.querySelectorAll('input[name^="batchOauthGroupName-"]') ?? [],
    ) as HTMLInputElement[];
    expect(groupInputs[0]?.value).toBe("beta");
    expect(groupInputs[1]?.value).toBe("custom");
    expect(saveAccount).toHaveBeenCalledTimes(1);
    expect(saveAccount).toHaveBeenCalledWith(
      41,
      expect.objectContaining({ groupName: "beta" }),
    );
  }, 10_000);

  it("does not reapply the header default group after a completed row opts out manually", async () => {
    const saveAccount = vi.fn().mockImplementation(
      async (accountId: number, payload: Record<string, unknown>) => ({
        id: accountId,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Inherited Row",
        groupName:
          typeof payload.groupName === "string" ? payload.groupName : "alpha",
        isMother: false,
        status: "active",
        enabled: true,
        duplicateInfo: null,
        history: [],
        note: "Seed note",
        tags: [],
        effectiveRoutingRule: {
          guardEnabled: false,
          allowCutOut: true,
          allowCutIn: true,
          sourceTagIds: [],
          sourceTagNames: [],
          guardRules: [],
        },
      }),
    );
    mockUpstreamAccounts({ saveAccount });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=batchOauth",
      state: {
        draft: {
          batchOauth: {
            defaultGroupName: "alpha",
            rows: [
              buildCompletedBatchOauthRow({
                id: "row-1",
                displayName: "Inherited Row",
                groupName: "alpha",
                metadataPersisted: {
                  displayName: "Inherited Row",
                  groupName: "alpha",
                  note: "Seed note",
                  isMother: false,
                  tagIds: [],
                },
              }),
            ],
          },
        },
      },
    });
    await flushAsync();

    setComboboxValue('input[name="batchOauthGroupName-row-1"]', "custom");
    await flushAsync();

    setComboboxValue('input[name="batchOauthGroupName-row-1"]', "alpha");
    await flushAsync();

    setComboboxValue('input[name="batchOauthDefaultGroupName"]', "beta");
    await flushAsync();

    const groupInput = host?.querySelector(
      'input[name="batchOauthGroupName-row-1"]',
    );
    expect(groupInput).toHaveProperty("value", "alpha");
    expect(saveAccount).toHaveBeenCalledTimes(2);
    expect(saveAccount).toHaveBeenNthCalledWith(
      2,
      41,
      expect.objectContaining({ groupName: "alpha" }),
    );
  }, 10_000);

  it("preserves the mother flag when a completed mother row changes groups", async () => {
    const saveAccount = vi.fn().mockImplementation(
      async (accountId: number, payload: Record<string, unknown>) => ({
        id: accountId,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Row One",
        groupName:
          typeof payload.groupName === "string" ? payload.groupName : "prod",
        isMother: payload.isMother === true,
        status: "active",
        enabled: true,
        duplicateInfo: null,
        history: [],
        note: "Seed note",
        tags: [],
        effectiveRoutingRule: {
          guardEnabled: false,
          allowCutOut: true,
          allowCutIn: true,
          sourceTagIds: [],
          sourceTagNames: [],
          guardRules: [],
        },
      }),
    );
    mockUpstreamAccounts({ saveAccount });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=batchOauth",
      state: {
        draft: {
          batchOauth: {
            rows: [
              buildCompletedBatchOauthRow({
                isMother: true,
                metadataPersisted: {
                  displayName: "Row One",
                  groupName: "prod",
                  note: "Seed note",
                  isMother: true,
                  tagIds: [],
                },
              }),
            ],
          },
        },
      },
    });
    await flushAsync();

    expect(findButton(/Toggle mother account/i)?.getAttribute("aria-pressed")).toBe(
      "true",
    );

    setComboboxValue('input[name="batchOauthGroupName-row-1"]', "analytics");
    await flushAsync();

    expect(saveAccount).toHaveBeenCalledWith(
      41,
      expect.objectContaining({
        groupName: "analytics",
        isMother: true,
      }),
    );
    expect(pageTextContent()).not.toContain("Mother updated");
  }, 10_000);

  it("demotes sibling completed rows after a mother reassignment save", async () => {
    const saveAccount = vi.fn().mockImplementation(
      async (accountId: number, payload: Record<string, unknown>) => ({
        id: accountId,
        kind: "oauth_codex",
        provider: "codex",
        displayName: accountId === 41 ? "Row One" : "Row Two",
        groupName:
          typeof payload.groupName === "string" ? payload.groupName : "prod",
        isMother: payload.isMother === true,
        status: "active",
        enabled: true,
        duplicateInfo: null,
        history: [],
        note: typeof payload.note === "string" ? payload.note : "Seed note",
        tags: [],
        effectiveRoutingRule: {
          guardEnabled: false,
          allowCutOut: true,
          allowCutIn: true,
          sourceTagIds: [],
          sourceTagNames: [],
          guardRules: [],
        },
      }),
    );
    mockUpstreamAccounts({ saveAccount });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=batchOauth",
      state: {
        draft: {
          batchOauth: {
            rows: [
              buildCompletedBatchOauthRow({
                id: "row-1",
                displayName: "Row One",
                isMother: false,
                metadataPersisted: {
                  displayName: "Row One",
                  groupName: "prod",
                  note: "Seed note",
                  isMother: false,
                  tagIds: [],
                },
              }),
              buildCompletedBatchOauthRow({
                id: "row-2",
                displayName: "Row Two",
                isMother: true,
                session: {
                  loginId: "login-2",
                  status: "completed",
                  authUrl: null,
                  redirectUri: null,
                  expiresAt: "2026-03-13T10:00:00.000Z",
                  accountId: 42,
                  error: null,
                },
                metadataPersisted: {
                  displayName: "Row Two",
                  groupName: "prod",
                  note: "Seed note",
                  isMother: true,
                  tagIds: [],
                },
              }),
            ],
          },
        },
      },
    });
    await flushAsync();

    const motherButtons = Array.from(host?.querySelectorAll("button") ?? []).filter(
      (candidate): candidate is HTMLButtonElement =>
        candidate instanceof HTMLButtonElement &&
        candidate.getAttribute("aria-label") === "Toggle mother account",
    );
    expect(motherButtons).toHaveLength(2);

    act(() => {
      motherButtons[0]?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushAsync();

    expect(motherButtons[0]?.getAttribute("aria-pressed")).toBe("true");
    expect(motherButtons[1]?.getAttribute("aria-pressed")).toBe("false");

    setInputValue('input[name="batchOauthNote-row-2"]', "Row Two updated");
    blurField('input[name="batchOauthNote-row-2"]');
    await flushAsync();

    expect(saveAccount).toHaveBeenNthCalledWith(
      2,
      42,
      expect.objectContaining({
        note: "Row Two updated",
        isMother: false,
      }),
    );
  }, 10_000);

  it("uses the saved account tags as the completed-row baseline when metadata state is restored", async () => {
    const saveAccount = vi.fn().mockImplementation(
      async (accountId: number, payload: Record<string, unknown>) => ({
        id: accountId,
        kind: "oauth_codex",
        provider: "codex",
        displayName:
          typeof payload.displayName === "string"
            ? payload.displayName
            : "Row One",
        groupName: "prod",
        isMother: false,
        status: "active",
        enabled: true,
        duplicateInfo: null,
        history: [],
        note: "Seed note",
        tags: Array.isArray(payload.tagIds)
          ? payload.tagIds.map((tagId) => ({
              id: Number(tagId),
              name: tagId === 2 ? "burst-safe" : `tag-${tagId}`,
              routingRule: {
                guardEnabled: false,
                allowCutOut: true,
                allowCutIn: true,
              },
            }))
          : [],
        effectiveRoutingRule: {
          guardEnabled: false,
          allowCutOut: true,
          allowCutIn: true,
          sourceTagIds: [],
          sourceTagNames: [],
          guardRules: [],
        },
      }),
    );
    mockUpstreamAccounts({
      saveAccount,
      items: [
        {
          id: 41,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Row One",
          groupName: "prod",
          isMother: false,
          status: "active",
          enabled: true,
          tags: [
            {
              id: 2,
              name: "burst-safe",
              routingRule: {
                guardEnabled: false,
                allowCutOut: true,
                allowCutIn: true,
              },
            },
          ],
          effectiveRoutingRule: {
            guardEnabled: false,
            allowCutOut: true,
            allowCutIn: true,
            sourceTagIds: [],
            sourceTagNames: [],
            guardRules: [],
          },
        },
      ],
    });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=batchOauth",
      state: {
        draft: {
          batchOauth: {
            tagIds: [2],
            rows: [
              buildCompletedBatchOauthRow({
                metadataPersisted: null,
              }),
            ],
          },
        },
      },
    });
    await flushAsync();

    setInputValue('input[name="batchOauthDisplayName-row-1"]', "Row One Restored");
    blurField('input[name="batchOauthDisplayName-row-1"]');
    await flushAsync();

    expect(saveAccount).toHaveBeenCalledTimes(1);
    expect(saveAccount).toHaveBeenCalledWith(
      41,
      expect.objectContaining({
        displayName: "Row One Restored",
        tagIds: [2],
      }),
    );
  }, 10_000);

  it("syncs restored completed rows to the initial shared tag set", async () => {
    const saveAccount = vi.fn().mockImplementation(
      async (accountId: number, payload: Record<string, unknown>) => ({
        id: accountId,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Tagged Row",
        groupName: "prod",
        isMother: false,
        status: "active",
        enabled: true,
        duplicateInfo: null,
        history: [],
        note: "Seed note",
        tags: Array.isArray(payload.tagIds)
          ? payload.tagIds.map((tagId) => ({
              id: Number(tagId),
              name: tagId === 1 ? "vip" : `tag-${tagId}`,
              routingRule: {
                guardEnabled: false,
                allowCutOut: true,
                allowCutIn: true,
              },
            }))
          : [],
        effectiveRoutingRule: {
          guardEnabled: false,
          allowCutOut: true,
          allowCutIn: true,
          sourceTagIds: [],
          sourceTagNames: [],
          guardRules: [],
        },
      }),
    );
    mockUpstreamAccounts({ saveAccount });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=batchOauth",
      state: {
        draft: {
          batchOauth: {
            tagIds: [1],
            rows: [
              buildCompletedBatchOauthRow({
                displayName: "Tagged Row",
                metadataPersisted: null,
              }),
            ],
          },
        },
      },
    });
    await flushAsync();

    expect(saveAccount).toHaveBeenCalledTimes(1);
    expect(saveAccount).toHaveBeenCalledWith(
      41,
      expect.objectContaining({ tagIds: [1] }),
    );
  }, 10_000);

  it("does not clear completed-row tags when restored drafts never saved shared tags", async () => {
    const saveAccount = vi.fn().mockImplementation(
      async (accountId: number, payload: Record<string, unknown>) => ({
        id: accountId,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Tagged Row",
        groupName: "prod",
        isMother: false,
        status: "active",
        enabled: true,
        duplicateInfo: null,
        history: [],
        note: "Seed note",
        tags: Array.isArray(payload.tagIds)
          ? payload.tagIds.map((tagId) => ({
              id: Number(tagId),
              name: tagId === 1 ? "vip" : `tag-${tagId}`,
              routingRule: {
                guardEnabled: false,
                allowCutOut: true,
                allowCutIn: true,
              },
            }))
          : [],
        effectiveRoutingRule: {
          guardEnabled: false,
          allowCutOut: true,
          allowCutIn: true,
          sourceTagIds: [],
          sourceTagNames: [],
          guardRules: [],
        },
      }),
    );
    mockUpstreamAccounts({
      saveAccount,
      items: [
        {
          id: 41,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Tagged Row",
          groupName: "prod",
          isMother: false,
          status: "active",
          enabled: true,
          tags: [
            {
              id: 2,
              name: "burst-safe",
              routingRule: {
                guardEnabled: false,
                allowCutOut: true,
                allowCutIn: true,
              },
            },
          ],
          effectiveRoutingRule: {
            guardEnabled: false,
            allowCutOut: true,
            allowCutIn: true,
            sourceTagIds: [],
            sourceTagNames: [],
            guardRules: [],
          },
        },
      ],
    });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=batchOauth",
      state: {
        draft: {
          batchOauth: {
            rows: [
              buildCompletedBatchOauthRow({
                displayName: "Tagged Row",
                metadataPersisted: null,
              }),
            ],
          },
        },
      },
    });
    await flushAsync();

    expect(saveAccount).not.toHaveBeenCalled();
  }, 10_000);

  it("syncs shared batch tags onto completed rows", async () => {
    const saveAccount = vi.fn().mockImplementation(
      async (accountId: number, payload: Record<string, unknown>) => ({
        id: accountId,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Tagged Row",
        groupName: "prod",
        isMother: false,
        status: "active",
        enabled: true,
        duplicateInfo: null,
        history: [],
        note: null,
        tags: Array.isArray(payload.tagIds)
          ? payload.tagIds.map((tagId) => ({
              id: Number(tagId),
              name: tagId === 1 ? "vip" : `tag-${tagId}`,
              routingRule: {
                guardEnabled: false,
                allowCutOut: true,
                allowCutIn: true,
              },
            }))
          : [],
        effectiveRoutingRule: {
          guardEnabled: false,
          allowCutOut: true,
          allowCutIn: true,
          sourceTagIds: [],
          sourceTagNames: [],
          guardRules: [],
        },
      }),
    );
    mockUpstreamAccounts({ saveAccount });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=batchOauth",
      state: {
        draft: {
          batchOauth: {
            rows: [
              buildCompletedBatchOauthRow({
                displayName: "Tagged Row",
              }),
            ],
          },
        },
      },
    });
    await flushAsync();

    clickButton(/Add tag/i);
    await flushAsync();
    const vipOption = Array.from(document.body.querySelectorAll("[cmdk-item]")).find(
      (candidate) => (candidate.textContent || "").includes("vip"),
    );
    if (!(vipOption instanceof HTMLElement)) {
      throw new Error("missing vip tag option");
    }
    act(() => {
      vipOption.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushAsync();

    expect(saveAccount).toHaveBeenCalledTimes(1);
    expect(saveAccount).toHaveBeenCalledWith(
      41,
      expect.objectContaining({ tagIds: [1] }),
    );
  }, 10_000);

  it("retries completed-row shared tag sync after a transient save failure", async () => {
    let firstRowFailures = 0;
    const saveAccount = vi.fn().mockImplementation(
      async (accountId: number, payload: Record<string, unknown>) => {
        if (
          accountId === 41 &&
          Array.isArray(payload.tagIds) &&
          payload.tagIds.length === 1 &&
          payload.tagIds[0] === 1 &&
          firstRowFailures === 0
        ) {
          firstRowFailures += 1;
          throw new Error("Transient tag sync failure");
        }
        return {
          id: accountId,
          kind: "oauth_codex",
          provider: "codex",
          displayName: accountId === 41 ? "Row One" : "Row Two",
          groupName: "prod",
          isMother: false,
          status: "active",
          enabled: true,
          duplicateInfo: null,
          history: [],
          note: null,
          tags: Array.isArray(payload.tagIds)
            ? payload.tagIds.map((tagId) => ({
                id: Number(tagId),
                name: tagId === 1 ? "vip" : `tag-${tagId}`,
                routingRule: {
                  guardEnabled: false,
                  allowCutOut: true,
                  allowCutIn: true,
                },
              }))
            : [],
          effectiveRoutingRule: {
            guardEnabled: false,
            allowCutOut: true,
            allowCutIn: true,
            sourceTagIds: [],
            sourceTagNames: [],
            guardRules: [],
          },
        };
      },
    );
    mockUpstreamAccounts({ saveAccount });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=batchOauth",
      state: {
        draft: {
          batchOauth: {
            rows: [
              buildCompletedBatchOauthRow({
                displayName: "Row One",
                metadataPersisted: {
                  displayName: "Row One",
                  groupName: "prod",
                  note: "Seed note",
                  isMother: false,
                  tagIds: [],
                },
              }),
              buildCompletedBatchOauthRow({
                id: "row-2",
                displayName: "Row Two",
                session: {
                  loginId: "login-2",
                  status: "completed",
                  authUrl: null,
                  redirectUri: null,
                  expiresAt: "2026-03-13T10:00:00.000Z",
                  accountId: 42,
                  error: null,
                },
                metadataPersisted: {
                  displayName: "Row Two",
                  groupName: "prod",
                  note: "Seed note",
                  isMother: false,
                  tagIds: [],
                },
              }),
            ],
          },
        },
      },
    });
    await flushAsync();

    clickButton(/Add tag/i);
    await flushAsync();
    const vipOption = Array.from(document.body.querySelectorAll("[cmdk-item]")).find(
      (candidate) => (candidate.textContent || "").includes("vip"),
    );
    if (!(vipOption instanceof HTMLElement)) {
      throw new Error("missing vip tag option");
    }
    act(() => {
      vipOption.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushAsync();
    await flushAsync();

    expect(
      saveAccount.mock.calls.filter(([accountId]) => accountId === 41),
    ).toHaveLength(2);
    expect(
      saveAccount.mock.calls.filter(([accountId]) => accountId === 42),
    ).toHaveLength(1);
    expect(pageTextContent()).not.toContain("Transient tag sync failure");
  }, 10_000);

  it("keeps the mother toggle reverted when completed-row persistence fails", async () => {
    const saveAccount = vi.fn().mockRejectedValue(
      new Error("Mother save failed"),
    );
    mockUpstreamAccounts({ saveAccount });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=batchOauth",
      state: {
        draft: {
          batchOauth: {
            rows: [buildCompletedBatchOauthRow()],
          },
        },
      },
    });
    await flushAsync();

    const motherToggleBefore = findButton(/Toggle mother account/i);
    expect(motherToggleBefore?.getAttribute("aria-pressed")).toBe("false");

    clickButton(/Toggle mother account/i);
    await flushAsync();

    expect(saveAccount).toHaveBeenCalledWith(
      41,
      expect.objectContaining({ isMother: true }),
    );
    expect(findButton(/Toggle mother account/i)?.getAttribute("aria-pressed")).toBe(
      "false",
    );
    expect(pageTextContent()).toContain("Mother save failed");
  }, 10_000);

  it("keeps completed-row save failures isolated to the failing row", async () => {
    const saveAccount = vi.fn().mockImplementation(
      async (accountId: number, payload: Record<string, unknown>) => {
        if (accountId === 41) {
          throw new Error("Save failed for row one");
        }
        return {
          id: accountId,
          kind: "oauth_codex",
          provider: "codex",
          displayName:
            typeof payload.displayName === "string"
              ? payload.displayName
              : "Row Two",
          groupName: "prod",
          isMother: false,
          status: "active",
          enabled: true,
          duplicateInfo: null,
          history: [],
          note: null,
          tags: [],
          effectiveRoutingRule: {
            guardEnabled: false,
            allowCutOut: true,
            allowCutIn: true,
            sourceTagIds: [],
            sourceTagNames: [],
            guardRules: [],
          },
        };
      },
    );
    mockUpstreamAccounts({ saveAccount });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=batchOauth",
      state: {
        draft: {
          batchOauth: {
            rows: [
              buildCompletedBatchOauthRow({
                id: "row-1",
                session: {
                  loginId: "login-1",
                  status: "completed",
                  authUrl: null,
                  redirectUri: null,
                  expiresAt: "2026-03-13T10:00:00.000Z",
                  accountId: 41,
                  error: null,
                },
              }),
              buildCompletedBatchOauthRow({
                id: "row-2",
                displayName: "Row Two",
                session: {
                  loginId: "login-2",
                  status: "completed",
                  authUrl: null,
                  redirectUri: null,
                  expiresAt: "2026-03-13T10:00:00.000Z",
                  accountId: 42,
                  error: null,
                },
                metadataPersisted: {
                  displayName: "Row Two",
                  groupName: "prod",
                  note: "Seed note",
                  isMother: false,
                  tagIds: [],
                },
              }),
            ],
          },
        },
      },
    });
    await flushAsync();

    setInputValue('input[name="batchOauthDisplayName-row-1"]', "Row One Broken");
    blurField('input[name="batchOauthDisplayName-row-1"]');
    await flushAsync();

    expect(pageTextContent()).toContain("Save failed for row one");

    setInputValue('input[name="batchOauthDisplayName-row-2"]', "Row Two Saved");
    blurField('input[name="batchOauthDisplayName-row-2"]');
    await flushAsync();

    expect(saveAccount).toHaveBeenCalledTimes(2);
    expect(saveAccount).toHaveBeenNthCalledWith(
      2,
      42,
      expect.objectContaining({ displayName: "Row Two Saved" }),
    );
    expect(
      host?.querySelector('input[name="batchOauthDisplayName-row-2"]'),
    ).toHaveProperty("value", "Row Two Saved");
  }, 10_000);

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
  }, 10_000);

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

    expect(pageTextContent()).toContain("Display name must be unique.");
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
    mockUpstreamAccounts({
      beginOauthMailboxSessionForAddress,
      beginOauthLogin,
    });
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
      mailboxChip?.dispatchEvent(
        new MouseEvent("mouseenter", { bubbles: true }),
      );
      mailboxChip?.dispatchEvent(
        new MouseEvent("mouseover", { bubbles: true }),
      );
    });
    await flushAsync();

    const editMailboxButton = document.body.querySelector(
      'button[title="Edit mailbox"]',
    );
    if (!(editMailboxButton instanceof HTMLButtonElement)) {
      throw new Error("missing edit mailbox button");
    }
    act(() => {
      editMailboxButton.dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
    });
    await flushAsync();
    setBodyInputValue(
      'input[name="batchOauthMailboxEditor-row-1"]',
      "edited-batch@mail-tw.707079.xyz",
    );
    clickBodyButton(/Submit mailbox/i);
    await flushAsync();

    expect(beginOauthMailboxSessionForAddress).toHaveBeenCalledWith(
      "edited-batch@mail-tw.707079.xyz",
    );
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

  it("auto-creates a supported mailbox from the batch popover editor when moemail is missing it", async () => {
    const beginOauthMailboxSessionForAddress = vi.fn().mockResolvedValue({
      supported: true,
      sessionId: "mailbox-generated-row-1",
      emailAddress: "finance.lab.d5r@mail-tw.707079.xyz",
      expiresAt: "2026-04-13T10:00:00.000Z",
      source: "generated",
    });
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-batch-generated-mailbox",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=batch-generated-mailbox",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({
      beginOauthMailboxSessionForAddress,
      beginOauthLogin,
    });
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
      mailboxChip?.dispatchEvent(
        new MouseEvent("mouseenter", { bubbles: true }),
      );
      mailboxChip?.dispatchEvent(
        new MouseEvent("mouseover", { bubbles: true }),
      );
    });
    await flushAsync();

    const editMailboxButton = document.body.querySelector(
      'button[title="Edit mailbox"]',
    );
    if (!(editMailboxButton instanceof HTMLButtonElement)) {
      throw new Error("missing edit mailbox button");
    }
    act(() => {
      editMailboxButton.dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
    });
    await flushAsync();
    setBodyInputValue(
      'input[name="batchOauthMailboxEditor-row-1"]',
      "finance.lab.d5r@mail-tw.707079.xyz",
    );
    clickBodyButton(/Submit mailbox/i);
    await flushAsync();

    expect(beginOauthMailboxSessionForAddress).toHaveBeenCalledWith(
      "finance.lab.d5r@mail-tw.707079.xyz",
    );
    expect(host?.textContent).toContain("finance.lab.d5r@mail-tw.707079.xyz");
    expect(host?.textContent).toContain("Generated mailbox");

    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    expect(beginOauthLogin).toHaveBeenCalledWith({
      displayName: "Batch Row",
      groupName: "prod",
      note: undefined,
      tagIds: [],
      groupNote: undefined,
      isMother: false,
      mailboxSessionId: "mailbox-generated-row-1",
      mailboxAddress: "finance.lab.d5r@mail-tw.707079.xyz",
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
    mockUpstreamAccounts({
      beginOauthLogin,
      beginOauthMailboxSessionForAddress,
    });
    render("/account-pool/upstream-accounts/new?mode=batchOauth");
    await flushAsync();

    setInputValue('input[name^="batchOauthDisplayName-"]', "Batch Unsupported");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    const mailboxChip = findBodyButton(/Edit mailbox/i);
    expect(mailboxChip).toBeInstanceOf(HTMLButtonElement);
    act(() => {
      mailboxChip?.dispatchEvent(
        new MouseEvent("mouseenter", { bubbles: true }),
      );
      mailboxChip?.dispatchEvent(
        new MouseEvent("mouseover", { bubbles: true }),
      );
    });
    await flushAsync();

    const editMailboxButton = document.body.querySelector(
      'button[title="Edit mailbox"]',
    );
    if (!(editMailboxButton instanceof HTMLButtonElement)) {
      throw new Error("missing edit mailbox button");
    }
    act(() => {
      editMailboxButton.dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
    });
    await flushAsync();
    setBodyInputValue(
      'input[name^="batchOauthMailboxEditor-"]',
      "unsupported@example.com",
    );
    clickBodyButton(/Submit mailbox/i);
    await flushAsync();

    expect(beginOauthMailboxSessionForAddress).toHaveBeenCalledWith(
      "unsupported@example.com",
    );
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
      mailboxChip?.dispatchEvent(
        new MouseEvent("mouseenter", { bubbles: true }),
      );
      mailboxChip?.dispatchEvent(
        new MouseEvent("mouseover", { bubbles: true }),
      );
    });
    await flushAsync();

    const editMailboxButton = document.body.querySelector(
      'button[title="Edit mailbox"]',
    );
    if (!(editMailboxButton instanceof HTMLButtonElement)) {
      throw new Error("missing edit mailbox button");
    }
    act(() => {
      editMailboxButton.dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
    });
    await flushAsync();

    setBodyInputValue(
      'input[name^="batchOauthMailboxEditor-"]',
      "not-an-email",
    );
    clickBodyButton(/Submit mailbox/i);
    await flushAsync();

    expect(beginOauthMailboxSessionForAddress).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain(
      "Enter a valid email address before attaching it.",
    );
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
      mailboxChip?.dispatchEvent(
        new MouseEvent("mouseenter", { bubbles: true }),
      );
      mailboxChip?.dispatchEvent(
        new MouseEvent("mouseover", { bubbles: true }),
      );
    });
    await flushAsync();

    const editMailboxButton = document.body.querySelector(
      'button[title="Edit mailbox"]',
    );
    if (!(editMailboxButton instanceof HTMLButtonElement)) {
      throw new Error("missing edit mailbox button");
    }
    act(() => {
      editMailboxButton.dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
    });
    await flushAsync();
    setBodyInputValue(
      'input[name="batchOauthMailboxEditor-row-1"]',
      "edited-batch@mail-tw.707079.xyz",
    );
    clickBodyButton(/Cancel mailbox edit/i);
    await flushAsync();

    expect(host?.textContent).toContain("original-batch@mail-tw.707079.xyz");
    expect(host?.textContent).not.toContain("edited-batch@mail-tw.707079.xyz");
  });

  it("keeps an existing batch oauth URL after attaching a new supported mailbox", async () => {
    vi.useFakeTimers();
    const beginOauthMailboxSessionForAddress = vi.fn().mockResolvedValue({
      supported: true,
      sessionId: "mailbox-attached-row-1",
      emailAddress: "edited-batch@mail-tw.707079.xyz",
      expiresAt: "2026-04-13T10:00:00.000Z",
      source: "attached",
    });
    const updateOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-existing-row-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=existing-row-1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({
      beginOauthMailboxSessionForAddress,
      updateOauthLogin,
    });
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
                  authUrl:
                    "https://auth.openai.com/authorize?login=existing-row-1",
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
      mailboxChip?.dispatchEvent(
        new MouseEvent("mouseenter", { bubbles: true }),
      );
      mailboxChip?.dispatchEvent(
        new MouseEvent("mouseover", { bubbles: true }),
      );
    });
    await flushAsync();

    const editMailboxButton = document.body.querySelector(
      'button[title="Edit mailbox"]',
    );
    if (!(editMailboxButton instanceof HTMLButtonElement)) {
      throw new Error("missing edit mailbox button");
    }
    act(() => {
      editMailboxButton.dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
    });
    await flushAsync();
    setBodyInputValue(
      'input[name="batchOauthMailboxEditor-row-1"]',
      "edited-batch@mail-tw.707079.xyz",
    );
    clickBodyButton(/Submit mailbox/i);
    await flushAsync();
    await flushSessionSyncDebounce();

    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(false);
    expect(updateOauthLogin).toHaveBeenCalledWith("login-existing-row-1", {
      displayName: "Batch Row",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "mailbox-attached-row-1",
      mailboxAddress: "edited-batch@mail-tw.707079.xyz",
    });
    expect(updateOauthLogin.mock.lastCall?.[1]).not.toHaveProperty("groupNote");
  });
});

describe("UpstreamAccountCreatePage display name validation", () => {
  it("does not resend an existing shared group note when generating oauth for a grouped row", async () => {
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

    setComboboxValue('input[name="batchOauthDefaultGroupName"]', "prod");
    await flushAsync();

    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    expect(beginOauthLogin).toHaveBeenCalledWith({
      displayName: undefined,
      groupName: "prod",
      tagIds: [],
      groupNote: undefined,
      note: undefined,
      isMother: false,
    });
  });

  it("passes a draft shared group note when generating oauth for a new grouped row", async () => {
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

    setComboboxValue('input[name="batchOauthDefaultGroupName"]', "new-team");
    await flushAsync();

    clickButton(/Edit group note/i);
    await flushAsync();
    const draftGroupNoteField =
      document.body.querySelector("textarea") ??
      (() => {
        throw new Error("missing group note textarea");
      })();
    if (!(draftGroupNoteField instanceof HTMLTextAreaElement)) {
      throw new Error("missing group note textarea");
    }
    setFieldValue(draftGroupNoteField, "Draft shared group note");
    clickBodyButton(/Save changes/i);
    await flushAsync();

    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    expect(beginOauthLogin).toHaveBeenCalledWith({
      displayName: undefined,
      groupName: "new-team",
      tagIds: [],
      groupNote: "Draft shared group note",
      note: undefined,
      isMother: false,
    });
  });

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

    expect(pageTextContent()).toContain("Display name must be unique.");
    expect(findButton(/Generate OAuth URL/i)?.disabled).toBe(true);
    expect(findButton(/Complete OAuth login/i)?.disabled).toBe(true);
    expect(beginOauthLogin).not.toHaveBeenCalled();
  });

  it("keeps a pending single oauth session while syncing metadata changes", async () => {
    vi.useFakeTimers();
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    const updateOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ beginOauthLogin, updateOauthLogin });
    render();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(false);

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth Renamed");
    expect(updateOauthLogin).not.toHaveBeenCalled();
    await flushSessionSyncDebounce();

    expect(host?.textContent).not.toContain("Generate a fresh OAuth URL");
    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(false);
    expect(updateOauthLogin).toHaveBeenCalledWith("login-1", {
      displayName: "Fresh OAuth Renamed",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
    expect(updateOauthLogin.mock.lastCall?.[1]).not.toHaveProperty("groupNote");
  });

  it("syncs edits made while oauth url generation is still in flight", async () => {
    vi.useFakeTimers();
    let resolveBeginOauthLogin:
      | ((value: LoginSessionStatusResponse) => void)
      | undefined;
    const beginOauthLogin = vi.fn().mockImplementation(
      () =>
        new Promise<LoginSessionStatusResponse>((resolve) => {
          resolveBeginOauthLogin = resolve;
        }),
    );
    const updateOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ beginOauthLogin, updateOauthLogin });
    render();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth Edited");
    await flushAsync();
    expect(updateOauthLogin).not.toHaveBeenCalled();

    if (!resolveBeginOauthLogin) {
      throw new Error("missing begin oauth resolver");
    }
    const finishBeginOauthLogin = resolveBeginOauthLogin;
    await act(async () => {
      finishBeginOauthLogin({
        loginId: "login-1",
        status: "pending",
        authUrl: "https://auth.openai.com/authorize?login=1",
        redirectUri: "http://localhost:1455/oauth/callback",
        expiresAt: "2026-03-13T10:00:00.000Z",
        accountId: null,
        error: null,
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    await flushAsync();
    await flushSessionSyncDebounce();

    expect(updateOauthLogin).toHaveBeenCalledTimes(1);
    expect(updateOauthLogin).toHaveBeenCalledWith("login-1", {
      displayName: "Fresh OAuth Edited",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
    expect(updateOauthLogin.mock.lastCall?.[1]).not.toHaveProperty("groupNote");
  });

  it("starts pending single oauth metadata sync after the debounce window while typing", async () => {
    vi.useFakeTimers();
    const updateOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ updateOauthLogin });
    render();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth Immediate");
    await flushAsync();
    expect(updateOauthLogin).not.toHaveBeenCalled();
    await flushSessionSyncDebounce();

    expect(updateOauthLogin).toHaveBeenCalledTimes(1);
    expect(updateOauthLogin).toHaveBeenCalledWith("login-1", {
      displayName: "Fresh OAuth Immediate",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
    expect(updateOauthLogin.mock.lastCall?.[1]).not.toHaveProperty("groupNote");
  });

  it("waits for the latest single oauth metadata sync before copying the oauth url", async () => {
    vi.useFakeTimers();
    const writeText = vi.fn().mockResolvedValue(undefined);
    const originalClipboard = navigator.clipboard;
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText,
      },
    });
    let resolveSync:
      | ((value: LoginSessionStatusResponse) => void)
      | undefined;
    const firstSync = new Promise<LoginSessionStatusResponse>((resolve) => {
      resolveSync = resolve;
    });
    const updateOauthLogin = vi.fn().mockReturnValueOnce(firstSync);
    mockUpstreamAccounts({ updateOauthLogin });
    render();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth Copied");
    await flushAsync();
    expect(updateOauthLogin).not.toHaveBeenCalled();

    clickButton(/Copy OAuth URL/i);
    await flushAsync();

    expect(updateOauthLogin).toHaveBeenCalledWith("login-1", {
      displayName: "Fresh OAuth Copied",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
    expect(updateOauthLogin.mock.lastCall?.[1]).not.toHaveProperty("groupNote");
    expect(writeText).not.toHaveBeenCalled();

    if (!resolveSync) {
      throw new Error("missing oauth sync resolver");
    }
    resolveSync({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    await flushAsync();

    expect(writeText).toHaveBeenCalledWith(
      "https://auth.openai.com/authorize?login=1",
    );

    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: originalClipboard,
    });
  });

  it("does not copy a stale single oauth url after the session completes during sync flush", async () => {
    vi.useFakeTimers();
    const writeText = vi.fn().mockResolvedValue(undefined);
    const originalClipboard = navigator.clipboard;
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText,
      },
    });
    const updateOauthLogin = vi
      .fn()
      .mockRejectedValue(new Error("This login session can no longer be edited."));
    const getLoginSession = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "completed",
      authUrl: null,
      redirectUri: null,
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: 41,
      error: null,
    });
    mockUpstreamAccounts({ updateOauthLogin, getLoginSession });
    render();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth Completed");
    await flushAsync();
    clickButton(/Copy OAuth URL/i);
    await flushAsync();
    await flushAsync();

    expect(updateOauthLogin).toHaveBeenCalledWith("login-1", {
      displayName: "Fresh OAuth Completed",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
    expect(getLoginSession).toHaveBeenCalledWith("login-1");
    expect(writeText).not.toHaveBeenCalled();
    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(true);

    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: originalClipboard,
    });
  });

  it("falls back to the cached single oauth url when sync refresh fails transiently", async () => {
    vi.useFakeTimers();
    const writeText = vi.fn().mockResolvedValue(undefined);
    const originalClipboard = navigator.clipboard;
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText,
      },
    });
    const updateOauthLogin = vi.fn().mockRejectedValue(new Error("network dropped"));
    const getLoginSession = vi
      .fn()
      .mockRejectedValue(new Error("temporary status refresh failure"));
    mockUpstreamAccounts({ updateOauthLogin, getLoginSession });
    render();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth Retry");
    await flushAsync();
    clickButton(/Copy OAuth URL/i);
    await flushAsync();

    expect(updateOauthLogin).toHaveBeenCalledWith("login-1", {
      displayName: "Fresh OAuth Retry",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
    expect(getLoginSession).toHaveBeenCalledWith("login-1");
    expect(writeText).toHaveBeenCalledWith(
      "https://auth.openai.com/authorize?login=1",
    );

    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: originalClipboard,
    });
  });

  it("does not copy a cached single oauth url after a non-retryable sync failure", async () => {
    vi.useFakeTimers();
    const writeText = vi.fn().mockResolvedValue(undefined);
    const originalClipboard = navigator.clipboard;
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText,
      },
    });
    const updateOauthLogin = vi
      .fn()
      .mockRejectedValue(new Error("Request failed: 409 duplicate displayName"));
    const getLoginSession = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ updateOauthLogin, getLoginSession });
    render();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    setInputValue('input[name="oauthDisplayName"]', "Duplicate OAuth");
    await flushAsync();
    clickButton(/Copy OAuth URL/i);
    await flushAsync();

    expect(updateOauthLogin).toHaveBeenCalled();
    expect(writeText).not.toHaveBeenCalled();
    expect(pageTextContent()).toContain("Request failed: 409 duplicate displayName");

    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: originalClipboard,
    });
  });

  it("does not send groupNote while syncing pending oauth metadata for an existing group", async () => {
    vi.useFakeTimers();
    const writeText = vi.fn().mockResolvedValue(undefined);
    const originalClipboard = navigator.clipboard;
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText,
      },
    });
    const updateOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      updatedAt: "2026-03-13T10:01:00.000Z",
      accountId: null,
      error: null,
    });
    const getLoginSession = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      updatedAt: "2026-03-13T10:01:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({
      groups: [{ groupName: "prod", note: "Saved note" }],
      updateOauthLogin,
      getLoginSession,
    });
    render();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth");
    await flushAsync();
    setComboboxValue('input[name="oauthGroupName"]', "prod");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth Retry");
    await flushAsync();
    clickButton(/Copy OAuth URL/i);
    await flushAsync();

    expect(updateOauthLogin).toHaveBeenCalled();
    expect(updateOauthLogin.mock.calls[0]?.[1]).toEqual({
      displayName: "Fresh OAuth Retry",
      groupName: "prod",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
    expect(writeText).toHaveBeenCalledWith(
      "https://auth.openai.com/authorize?login=1",
    );

    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: originalClipboard,
    });
  });

  it("dispatches an unload keepalive even while oauth metadata sync is already in flight", async () => {
    vi.useFakeTimers();
    let resolveFirstSync:
      | ((value: LoginSessionStatusResponse) => void)
      | undefined;
    const firstSync = new Promise<LoginSessionStatusResponse>((resolve) => {
      resolveFirstSync = resolve;
    });
    const updateOauthLogin = vi.fn().mockReturnValueOnce(firstSync);
    mockUpstreamAccounts({ updateOauthLogin });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      state: {
        draft: {
          oauth: {
            displayName: "Fresh OAuth",
            session: {
              loginId: "login-1",
              status: "pending",
              authUrl: "https://auth.openai.com/authorize?login=1",
              redirectUri: "http://localhost:1455/oauth/callback",
              expiresAt: "2026-03-13T10:00:00.000Z",
              updatedAt: "2026-03-13T09:55:00.000Z",
              accountId: null,
              error: null,
            },
            sessionHint: "OAuth URL ready",
          },
        },
      },
    });
    await flushAsync();
    await flushSessionSyncDebounce();

    expect(updateOauthLogin).toHaveBeenCalledTimes(1);

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth Latest");
    await flushAsync();

    act(() => {
      window.dispatchEvent(new Event("pagehide"));
    });
    await flushAsync();

    expect(apiMocks.updateOauthLoginSessionKeepalive).toHaveBeenCalledWith(
      "login-1",
      {
        displayName: "Fresh OAuth Latest",
        groupName: "",
        note: "",
        tagIds: [],
        isMother: false,
        mailboxSessionId: "",
        mailboxAddress: "",
      },
      "2026-03-13T09:55:00.000Z",
    );

    if (!resolveFirstSync) {
      throw new Error("missing oauth sync resolver");
    }
  });

  it("dispatches the latest oauth metadata when pagehide fires in the same act as an edit", async () => {
    vi.useFakeTimers();
    mockUpstreamAccounts({ updateOauthLogin: vi.fn() });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      state: {
        draft: {
          oauth: {
            displayName: "Fresh OAuth",
            session: {
              loginId: "login-1",
              status: "pending",
              authUrl: "https://auth.openai.com/authorize?login=1",
              redirectUri: "http://localhost:1455/oauth/callback",
              expiresAt: "2026-03-13T10:00:00.000Z",
              updatedAt: "2026-03-13T09:55:00.000Z",
              accountId: null,
              error: null,
            },
            sessionHint: "OAuth URL ready",
          },
        },
      },
    });

    await flushAsync();
    const input = host?.querySelector('input[name="oauthDisplayName"]');
    if (!(input instanceof HTMLInputElement)) {
      throw new Error("missing input: oauthDisplayName");
    }
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )?.set;
    if (!setter) {
      throw new Error("missing native setter: oauthDisplayName");
    }

    act(() => {
      setter.call(input, "Fresh OAuth Pagehide");
      input.dispatchEvent(new Event("input", { bubbles: true }));
      input.dispatchEvent(new Event("change", { bubbles: true }));
      window.dispatchEvent(new Event("pagehide"));
    });

    expect(apiMocks.updateOauthLoginSessionKeepalive).toHaveBeenCalledWith(
      "login-1",
      {
        displayName: "Fresh OAuth Pagehide",
        groupName: "",
        note: "",
        tagIds: [],
        isMother: false,
        mailboxSessionId: "",
        mailboxAddress: "",
      },
      "2026-03-13T09:55:00.000Z",
    );
  });

  it("dispatches a keepalive for pending oauth metadata when the create page unmounts", async () => {
    vi.useFakeTimers();
    const updateOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      updatedAt: "2026-03-13T09:56:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ updateOauthLogin });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      state: {
        draft: {
          oauth: {
            displayName: "Fresh OAuth",
            session: {
              loginId: "login-1",
              status: "pending",
              authUrl: "https://auth.openai.com/authorize?login=1",
              redirectUri: "http://localhost:1455/oauth/callback",
              expiresAt: "2026-03-13T10:00:00.000Z",
              updatedAt: "2026-03-13T09:55:00.000Z",
              accountId: null,
              error: null,
            },
            sessionHint: "OAuth URL ready",
          },
        },
      },
    });

    await flushAsync();
    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth Unmount");
    await flushAsync();
    expect(updateOauthLogin).not.toHaveBeenCalled();

    act(() => {
      root?.unmount();
      root = null;
    });

    expect(apiMocks.updateOauthLoginSessionKeepalive).toHaveBeenCalledWith(
      "login-1",
      {
        displayName: "Fresh OAuth Unmount",
        groupName: "",
        note: "",
        tagIds: [],
        isMother: false,
        mailboxSessionId: "",
        mailboxAddress: "",
      },
      "2026-03-13T09:55:00.000Z",
    );
  });

  it("emits an upstream account refresh when metadata sync finishes after callback completion", async () => {
    vi.useFakeTimers();
    const updateOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "completed",
      authUrl: null,
      redirectUri: null,
      expiresAt: "2026-03-13T10:00:00.000Z",
      updatedAt: "2026-03-13T09:56:00.000Z",
      accountId: 41,
      error: null,
    });
    mockUpstreamAccounts({ updateOauthLogin });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      state: {
        draft: {
          oauth: {
            displayName: "Fresh OAuth",
            session: {
              loginId: "login-1",
              status: "pending",
              authUrl: "https://auth.openai.com/authorize?login=1",
              redirectUri: "http://localhost:1455/oauth/callback",
              expiresAt: "2026-03-13T10:00:00.000Z",
              accountId: null,
              error: null,
            },
            sessionHint: "OAuth URL ready",
          },
        },
      },
    });

    await flushAsync();
    await flushSessionSyncDebounce();

    expect(updateOauthLogin).toHaveBeenCalled();
    expect(upstreamAccountsEventMocks.emitUpstreamAccountsChanged).toHaveBeenCalledTimes(
      1,
    );
  });

  it("does not surface a sync error when oauth completion wins the race", async () => {
    vi.useFakeTimers();
    const updateOauthLogin = vi
      .fn()
      .mockRejectedValue(new Error("Display name must be unique."));
    const getLoginSession = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "completed",
      authUrl: null,
      redirectUri: null,
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: 41,
      error: null,
    });
    mockUpstreamAccounts({ updateOauthLogin, getLoginSession });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      state: {
        draft: {
          oauth: {
            displayName: "Fresh OAuth",
            session: {
              loginId: "login-1",
              status: "pending",
              authUrl: "https://auth.openai.com/authorize?login=1",
              redirectUri: "http://localhost:1455/oauth/callback",
              expiresAt: "2026-03-13T10:00:00.000Z",
              accountId: null,
              error: null,
            },
            sessionHint: "OAuth URL ready",
          },
        },
      },
    });

    await flushAsync();
    setInputValue('input[name="oauthDisplayName"]', "Taken OAuth Name");
    await flushAsync();
    await flushSessionSyncDebounce();

    expect(getLoginSession).toHaveBeenCalledWith("login-1");
    expect(document.body.textContent).not.toContain(
      "Display name must be unique.",
    );
    expect(upstreamAccountsEventMocks.emitUpstreamAccountsChanged).toHaveBeenCalledTimes(
      1,
    );
  });

  it("uses the latest pending session updatedAt as the next oauth sync baseline", async () => {
    vi.useFakeTimers();
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      updatedAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    const updateOauthLogin = vi
      .fn()
      .mockResolvedValueOnce({
        loginId: "login-1",
        status: "pending",
        authUrl: "https://auth.openai.com/authorize?login=1",
        redirectUri: "http://localhost:1455/oauth/callback",
        expiresAt: "2026-03-13T10:00:00.000Z",
        updatedAt: "2026-03-13T10:01:00.000Z",
        accountId: null,
        error: null,
      })
      .mockResolvedValueOnce({
        loginId: "login-1",
        status: "pending",
        authUrl: "https://auth.openai.com/authorize?login=1",
        redirectUri: "http://localhost:1455/oauth/callback",
        expiresAt: "2026-03-13T10:00:00.000Z",
        updatedAt: "2026-03-13T10:02:00.000Z",
        accountId: null,
        error: null,
      });
    mockUpstreamAccounts({ beginOauthLogin, updateOauthLogin });
    render();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth A");
    await flushAsync();
    await flushSessionSyncDebounce();

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth B");
    await flushAsync();
    await flushSessionSyncDebounce();

    expect(updateOauthLogin).toHaveBeenNthCalledWith(1, "login-1", {
      displayName: "Fresh OAuth A",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    }, "2026-03-13T10:00:00.000Z");
    expect(updateOauthLogin.mock.calls[0]?.[1]).not.toHaveProperty("groupNote");
    expect(updateOauthLogin).toHaveBeenNthCalledWith(2, "login-1", {
      displayName: "Fresh OAuth B",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    }, "2026-03-13T10:01:00.000Z");
    expect(updateOauthLogin.mock.calls[1]?.[1]).not.toHaveProperty("groupNote");
  });

  it("coalesces pending single oauth sync requests while an earlier patch is in flight", async () => {
    vi.useFakeTimers();
    let resolveFirstSync:
      | ((value: LoginSessionStatusResponse) => void)
      | undefined;
    const firstSync = new Promise<LoginSessionStatusResponse>((resolve) => {
      resolveFirstSync = resolve;
    });
    const updateOauthLogin = vi
      .fn()
      .mockReturnValueOnce(firstSync)
      .mockResolvedValueOnce({
        loginId: "login-1",
        status: "pending",
        authUrl: "https://auth.openai.com/authorize?login=1",
        redirectUri: "http://localhost:1455/oauth/callback",
        expiresAt: "2026-03-13T10:00:00.000Z",
        accountId: null,
        error: null,
      });
    mockUpstreamAccounts({ updateOauthLogin });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      state: {
        draft: {
          oauth: {
            displayName: "Fresh OAuth",
            session: {
              loginId: "login-1",
              status: "pending",
              authUrl: "https://auth.openai.com/authorize?login=1",
              redirectUri: "http://localhost:1455/oauth/callback",
              expiresAt: "2026-03-13T10:00:00.000Z",
              accountId: null,
              error: null,
            },
            sessionHint: "OAuth URL ready",
          },
        },
      },
    });
    await flushAsync();
    await flushSessionSyncDebounce();

    expect(updateOauthLogin).toHaveBeenCalledTimes(1);

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth A");
    await flushAsync();
    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth AB");
    await flushAsync();
    await flushSessionSyncDebounce();

    expect(updateOauthLogin).toHaveBeenCalledTimes(1);

    if (!resolveFirstSync) {
      throw new Error("missing oauth sync resolver");
    }
    const finishFirstSync = resolveFirstSync;
    await act(async () => {
      finishFirstSync({
        loginId: "login-1",
        status: "pending",
        authUrl: "https://auth.openai.com/authorize?login=1",
        redirectUri: "http://localhost:1455/oauth/callback",
        expiresAt: "2026-03-13T10:00:00.000Z",
        accountId: null,
        error: null,
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    await flushAsync();

    expect(updateOauthLogin).toHaveBeenCalledTimes(2);
    expect(updateOauthLogin).toHaveBeenLastCalledWith("login-1", {
      displayName: "Fresh OAuth AB",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
    expect(updateOauthLogin.mock.lastCall?.[1]).not.toHaveProperty("groupNote");
  });

  it("does not auto-sync pending oauth drafts when writes are disabled", async () => {
    vi.useFakeTimers();
    const updateOauthLogin = vi.fn();
    mockUpstreamAccounts({ updateOauthLogin, writesEnabled: false });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      state: {
        draft: {
          oauth: {
            displayName: "Read Only OAuth",
            session: {
              loginId: "login-read-only",
              status: "pending",
              authUrl: "https://auth.openai.com/authorize?login=read-only",
              redirectUri: "http://localhost:1455/oauth/callback",
              expiresAt: "2026-03-13T10:00:00.000Z",
              accountId: null,
              error: null,
            },
            sessionHint: "OAuth URL ready",
          },
        },
      },
    });

    await flushAsync();
    await flushSessionSyncDebounce();
    await flushAsync();

    expect(updateOauthLogin).not.toHaveBeenCalled();
    expect(host?.textContent).not.toContain("cross-origin account writes are forbidden");
  });

  it("refreshes a dead pending single oauth session after metadata sync fails", async () => {
    vi.useFakeTimers();
    const updateOauthLogin = vi
      .fn()
      .mockRejectedValue(new Error("This login session can no longer be edited."));
    const getLoginSession = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "expired",
      authUrl: null,
      redirectUri: null,
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: "The login session has expired. Please create a new authorization link.",
    });
    mockUpstreamAccounts({ updateOauthLogin, getLoginSession });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      state: {
        draft: {
          oauth: {
            displayName: "Fresh OAuth",
            session: {
              loginId: "login-1",
              status: "pending",
              authUrl: "https://auth.openai.com/authorize?login=1",
              redirectUri: "http://localhost:1455/oauth/callback",
              expiresAt: "2026-03-13T10:00:00.000Z",
              accountId: null,
              error: null,
            },
            sessionHint: "OAuth URL ready",
          },
        },
      },
    });
    await flushAsync();
    await flushSessionSyncDebounce();

    expect(updateOauthLogin).toHaveBeenCalledWith("login-1", {
      displayName: "Fresh OAuth",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
    expect(getLoginSession).toHaveBeenCalledWith("login-1");
    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(true);
    expect(findButton(/Complete OAuth login/i)?.disabled).toBe(true);
    expect(host?.textContent).toContain(
      "The login session has expired. Please create a new authorization link.",
    );
  });

  it("does not surface a stale sync error after the oauth session already completed", async () => {
    vi.useFakeTimers();
    const updateOauthLogin = vi
      .fn()
      .mockRejectedValue(new Error("This login session can no longer be edited."));
    const getLoginSession = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "completed",
      authUrl: null,
      redirectUri: null,
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: 41,
      error: null,
    });
    mockUpstreamAccounts({ updateOauthLogin, getLoginSession });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      state: {
        draft: {
          oauth: {
            displayName: "Fresh OAuth",
            session: {
              loginId: "login-1",
              status: "pending",
              authUrl: "https://auth.openai.com/authorize?login=1",
              redirectUri: "http://localhost:1455/oauth/callback",
              expiresAt: "2026-03-13T10:00:00.000Z",
              accountId: null,
              error: null,
            },
            sessionHint: "OAuth URL ready",
          },
        },
      },
    });
    await flushAsync();
    await flushSessionSyncDebounce();

    expect(updateOauthLogin).toHaveBeenCalledWith("login-1", {
      displayName: "Fresh OAuth",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
    expect(getLoginSession).toHaveBeenCalledWith("login-1");
    expect(host?.textContent).not.toContain(
      "This login session can no longer be edited.",
    );
    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(true);
    expect(findButton(/Complete OAuth login/i)?.disabled).toBe(true);
  });

  it("syncs a restored pending oauth draft on first render", async () => {
    vi.useFakeTimers();
    const updateOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-restored-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=restored-1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ updateOauthLogin });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      state: {
        draft: {
          oauth: {
            displayName: "Restored OAuth Draft",
            groupName: "restored-group",
            note: "restored note",
            session: {
              loginId: "login-restored-1",
              status: "pending",
              authUrl: "https://auth.openai.com/authorize?login=restored-1",
              redirectUri: "http://localhost:1455/oauth/callback",
              expiresAt: "2026-03-13T10:00:00.000Z",
              accountId: null,
              error: null,
            },
            sessionHint: "OAuth URL ready",
          },
        },
      },
    });

    await flushAsync();
    await flushSessionSyncDebounce();
    await flushAsync();

    expect(updateOauthLogin).toHaveBeenCalledWith("login-restored-1", {
      displayName: "Restored OAuth Draft",
      groupName: "restored-group",
      note: "restored note",
      groupNote: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
  });

  it("invalidates pending relink sessions instead of live syncing metadata edits", async () => {
    vi.useFakeTimers();
    const updateOauthLogin = vi.fn();
    mockUpstreamAccounts({ updateOauthLogin });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?accountId=5",
      state: {
        draft: {
          oauth: {
            displayName: "Relink Draft",
            session: {
              loginId: "login-relink-1",
              status: "pending",
              authUrl: "https://auth.openai.com/authorize?login=relink-1",
              redirectUri: "http://localhost:1455/oauth/callback",
              expiresAt: "2026-03-13T10:00:00.000Z",
              accountId: 5,
              error: null,
            },
            sessionHint: "OAuth URL ready",
          },
        },
      },
    });
    await flushAsync();

    setInputValue('input[name="oauthDisplayName"]', "Relink Draft Updated");
    await flushSessionSyncDebounce();
    await flushAsync();

    expect(updateOauthLogin).not.toHaveBeenCalled();
    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(true);
    expect(host?.textContent).toContain(
      "Generate a fresh OAuth URL before completing login.",
    );
  });

  it("invalidates pending relink sessions when the mailbox binding changes", async () => {
    vi.useFakeTimers();
    const updateOauthLogin = vi.fn();
    mockUpstreamAccounts({ updateOauthLogin });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?accountId=5",
      state: {
        draft: {
          oauth: {
            displayName: "Relink Draft",
            mailboxSession: {
              supported: true,
              sessionId: "mailbox-relink-1",
              emailAddress: "linked@example.com",
              expiresAt: "2026-03-13T10:00:00.000Z",
              source: "attached",
            },
            mailboxInput: "linked@example.com",
            session: {
              loginId: "login-relink-1",
              status: "pending",
              authUrl: "https://auth.openai.com/authorize?login=relink-1",
              redirectUri: "http://localhost:1455/oauth/callback",
              expiresAt: "2026-03-13T10:00:00.000Z",
              accountId: 5,
              error: null,
            },
            sessionHint: "OAuth URL ready",
          },
        },
      },
    });
    await flushAsync();

    setInputValue('input[name="oauthMailboxInput"]', "different@example.com");
    await flushAsync();

    expect(updateOauthLogin).not.toHaveBeenCalled();
    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(true);
    expect(host?.textContent).toContain(
      "Generate a fresh OAuth URL before completing login.",
    );
    expect(host?.textContent).not.toContain("Attached mailbox");
  });

  it("retries an unchanged failed single oauth sync after a transient error", async () => {
    vi.useFakeTimers();
    const updateOauthLogin = vi
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
      .mockRejectedValueOnce(new Error("network dropped"))
      .mockResolvedValueOnce({
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
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ updateOauthLogin, getLoginSession });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      state: {
        draft: {
          oauth: {
            displayName: "Fresh OAuth",
            session: {
              loginId: "login-1",
              status: "pending",
              authUrl: "https://auth.openai.com/authorize?login=1",
              redirectUri: "http://localhost:1455/oauth/callback",
              expiresAt: "2026-03-13T10:00:00.000Z",
              accountId: null,
              error: null,
            },
            sessionHint: "OAuth URL ready",
          },
        },
      },
    });
    await flushAsync();
    await flushSessionSyncDebounce();

    expect(updateOauthLogin).toHaveBeenCalledTimes(1);

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth Retry");
    await flushAsync();
    expect(updateOauthLogin).toHaveBeenCalledTimes(1);
    await flushSessionSyncDebounce();
    expect(updateOauthLogin).toHaveBeenCalledTimes(2);

    await flushSessionSyncRetry();
    await flushAsync();

    expect(updateOauthLogin).toHaveBeenCalledTimes(3);
    expect(updateOauthLogin).toHaveBeenLastCalledWith("login-1", {
      displayName: "Fresh OAuth Retry",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });

    expect(getLoginSession).toHaveBeenCalledTimes(1);
  });

  it("retries the latest single oauth metadata after a stale sync fails during completion", async () => {
    vi.useFakeTimers();
    let rejectFirstSync: ((reason?: unknown) => void) | null = null;
    const firstSync = new Promise<{
      loginId: string;
      status: string;
      authUrl: string;
      redirectUri: string;
      expiresAt: string;
      accountId: null;
      error: null;
    }>((_resolve, reject) => {
      rejectFirstSync = reject;
    });
    const updateOauthLogin = vi
      .fn()
      .mockReturnValueOnce(firstSync)
      .mockResolvedValueOnce({
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
      displayName: "Fresh OAuth Valid",
    });
    const getLoginSession = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({
      updateOauthLogin,
      completeOauthLogin,
      getLoginSession,
    });
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

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth Invalid");
    await flushSessionSyncDebounce();
    await flushAsync();

    expect(updateOauthLogin).toHaveBeenCalledTimes(1);

    const displayNameInput = host?.querySelector('input[name="oauthDisplayName"]');
    const completeButton = findButton(/Complete OAuth login/i);
    if (!(displayNameInput instanceof HTMLInputElement) || !completeButton) {
      throw new Error("missing single oauth controls");
    }
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )?.set;
    if (!setter || !rejectFirstSync) {
      throw new Error("missing retry controls");
    }

    await act(async () => {
      setter.call(displayNameInput, "Fresh OAuth Valid");
      displayNameInput.dispatchEvent(new Event("input", { bubbles: true }));
      displayNameInput.dispatchEvent(new Event("change", { bubbles: true }));
      completeButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
      await Promise.resolve();
    });

    await act(async () => {
      rejectFirstSync?.(new Error("Display name must be unique."));
      await Promise.resolve();
      await Promise.resolve();
    });
    await flushAsync();
    await flushAsync();

    expect(updateOauthLogin).toHaveBeenCalledTimes(2);
    expect(updateOauthLogin).toHaveBeenNthCalledWith(2, "login-1", {
      displayName: "Fresh OAuth Valid",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
    expect(completeOauthLogin).toHaveBeenCalledWith("login-1", {
      callbackUrl: "http://localhost:1455/oauth/callback?code=test",
      mailboxSessionId: undefined,
      mailboxAddress: undefined,
    });
  });

  it("flushes the latest single oauth metadata before completing immediately after an edit", async () => {
    vi.useFakeTimers();
    const updateOauthLogin = vi.fn().mockResolvedValue({
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
      displayName: "Fresh OAuth Renamed",
    });
    mockUpstreamAccounts({ updateOauthLogin, completeOauthLogin });
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

    const displayNameInput = host?.querySelector('input[name="oauthDisplayName"]');
    const completeButton = findButton(/Complete OAuth login/i);
    if (!(displayNameInput instanceof HTMLInputElement) || !completeButton) {
      throw new Error("missing single oauth controls");
    }
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )?.set;
    if (!setter) {
      throw new Error("missing native input setter");
    }

    await act(async () => {
      setter.call(displayNameInput, "Fresh OAuth Renamed");
      displayNameInput.dispatchEvent(new Event("input", { bubbles: true }));
      displayNameInput.dispatchEvent(new Event("change", { bubbles: true }));
      completeButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
      await Promise.resolve();
    });
    await flushAsync();

    expect(updateOauthLogin).toHaveBeenCalledWith("login-1", {
      displayName: "Fresh OAuth Renamed",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
    expect(updateOauthLogin.mock.lastCall?.[1]).not.toHaveProperty("groupNote");
    expect(completeOauthLogin).toHaveBeenCalledWith("login-1", {
      callbackUrl: "http://localhost:1455/oauth/callback?code=test",
      mailboxSessionId: undefined,
      mailboxAddress: undefined,
    });
  });

  it("keeps pending batch oauth sessions when a draft group note changes", async () => {
    vi.useFakeTimers();
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    const updateOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ beginOauthLogin, updateOauthLogin });
    render("/account-pool/upstream-accounts/new?mode=batchOauth");

    setComboboxValue('input[name="batchOauthDefaultGroupName"]', "new-team");
    await flushAsync();

    clickButton(/Generate OAuth URL/i);
    await flushAsync();
    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(false);

    clickButton(/Edit group note/i);
    await flushAsync();
    const updatedGroupNoteField =
      document.body.querySelector("textarea") ??
      (() => {
        throw new Error("missing group note textarea");
      })();
    if (!(updatedGroupNoteField instanceof HTMLTextAreaElement)) {
      throw new Error("missing group note textarea");
    }
    setFieldValue(updatedGroupNoteField, "Updated draft shared note");
    clickBodyButton(/Save changes/i);
    await flushAsync();
    await flushSessionSyncDebounce();

    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(false);
    expect(updateOauthLogin).toHaveBeenCalledWith("login-1", {
      displayName: "",
      groupName: "new-team",
      note: "",
      groupNote: "Updated draft shared note",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
  });

  it("does not copy a stale batch oauth url after the session completes during sync flush", async () => {
    vi.useFakeTimers();
    const writeText = vi.fn().mockResolvedValue(undefined);
    const originalClipboard = navigator.clipboard;
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText,
      },
    });
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    const updateOauthLogin = vi
      .fn()
      .mockRejectedValue(new Error("This login session can no longer be edited."));
    const getLoginSession = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "completed",
      authUrl: null,
      redirectUri: null,
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: 41,
      error: null,
    });
    mockUpstreamAccounts({ beginOauthLogin, updateOauthLogin, getLoginSession });
    render("/account-pool/upstream-accounts/new?mode=batchOauth");

    setInputValue('input[name^="batchOauthDisplayName-"]', "Row One");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    setInputValue('input[name^="batchOauthDisplayName-"]', "Row One Completed");
    await flushAsync();
    clickButton(/Copy OAuth URL/i);
    await flushAsync();
    await flushAsync();

    expect(updateOauthLogin).toHaveBeenCalledWith("login-1", {
      displayName: "Row One Completed",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
    expect(getLoginSession).toHaveBeenCalledWith("login-1");
    expect(writeText).not.toHaveBeenCalled();
    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(true);

    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: originalClipboard,
    });
  });

  it("falls back to the cached batch oauth url when sync refresh fails transiently", async () => {
    vi.useFakeTimers();
    const writeText = vi.fn().mockResolvedValue(undefined);
    const originalClipboard = navigator.clipboard;
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText,
      },
    });
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    const updateOauthLogin = vi.fn().mockRejectedValue(new Error("network dropped"));
    const getLoginSession = vi
      .fn()
      .mockRejectedValue(new Error("temporary status refresh failure"));
    mockUpstreamAccounts({ beginOauthLogin, updateOauthLogin, getLoginSession });
    render("/account-pool/upstream-accounts/new?mode=batchOauth");

    setInputValue('input[name^="batchOauthDisplayName-"]', "Row One");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    setInputValue('input[name^="batchOauthDisplayName-"]', "Row One Retry");
    await flushAsync();
    clickButton(/Copy OAuth URL/i);
    await flushAsync();

    expect(updateOauthLogin).toHaveBeenCalledWith("login-1", {
      displayName: "Row One Retry",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
    expect(getLoginSession).toHaveBeenCalledWith("login-1");
    expect(writeText).toHaveBeenCalledWith(
      "https://auth.openai.com/authorize?login=1",
    );

    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: originalClipboard,
    });
  });

  it("does not copy a cached batch oauth url after a non-retryable sync failure", async () => {
    vi.useFakeTimers();
    const writeText = vi.fn().mockResolvedValue(undefined);
    const originalClipboard = navigator.clipboard;
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText,
      },
    });
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    const updateOauthLogin = vi
      .fn()
      .mockRejectedValue(new Error("Request failed: 422 invalid tags"));
    const getLoginSession = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ beginOauthLogin, updateOauthLogin, getLoginSession });
    render("/account-pool/upstream-accounts/new?mode=batchOauth");

    setInputValue('input[name^="batchOauthDisplayName-"]', "Row One");
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    setInputValue('input[name^="batchOauthDisplayName-"]', "Row One Retry");
    await flushAsync();
    clickButton(/Copy OAuth URL/i);
    await flushAsync();

    expect(updateOauthLogin).toHaveBeenCalled();
    expect(writeText).not.toHaveBeenCalled();
    expect(pageTextContent()).toContain("Request failed: 422 invalid tags");

    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: originalClipboard,
    });
  });

  it("blocks creating an API key account when the display name already exists", async () => {
    mockUpstreamAccounts();
    render();

    clickButton(/^API key$/i);
    setInputValue('input[name="apiKeyDisplayName"]', "Existing OAuth");
    await flushAsync();

    expect(pageTextContent()).toContain("Display name must be unique.");
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

    expect(document.body.textContent).toContain("Possible upstream duplicate");
    expect(document.body.textContent).toContain(
      "Matched: shared ChatGPT account id. Related account ids: 5.",
    );
    expect(navigateMock).not.toHaveBeenCalled();
  });

  it("clears a completed single oauth session after edits so a fresh url can be regenerated", async () => {
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

    expect(findButton(/Generate OAuth URL/i)?.disabled).toBe(true);

    setInputValue('input[name="oauthDisplayName"]', "Fresh OAuth Retry");
    await flushAsync();

    expect(findButton(/Generate OAuth URL/i)?.disabled).toBe(false);
    expect(pageTextContent()).toContain("Generate a fresh OAuth URL");
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
    expect(document.body.textContent).toContain("Display name must be unique.");
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
    mockUpstreamAccounts({
      beginOauthLogin,
      completeOauthLogin,
      getLoginSession,
    });
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
    mockUpstreamAccounts({
      beginOauthLogin,
      completeOauthLogin,
      getLoginSession,
    });
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

    expect(document.body.textContent).toContain("Possible upstream duplicate");
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

    const displayNameInput = setInputValue(
      'input[name="oauthDisplayName"]',
      "Manual Alias",
    );

    clickButton(/Generate/i);
    await flushAsync();

    expect(beginOauthMailboxSession).toHaveBeenCalledTimes(1);
    expect(displayNameInput.value).toBe("Manual Alias");
    expect(host?.textContent).toContain("temp-user-2@example.com");
  });

  it("keeps a pending oauth url when the mailbox draft changes before attach", async () => {
    vi.useFakeTimers();
    const beginOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    const updateOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-1",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=1",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ beginOauthLogin, updateOauthLogin });
    render("/account-pool/upstream-accounts/new?mode=oauth");

    clickButton(/Generate/i);
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    setInputValue('input[name="oauthMailboxInput"]', "new-target@example.com");
    await flushSessionSyncDebounce();

    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(false);
    expect(host?.textContent).not.toContain(
      "Generate a fresh OAuth URL before completing login.",
    );
    expect(updateOauthLogin).toHaveBeenCalledWith("login-1", {
      displayName: "mailbox-1@example.com",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
    expect(updateOauthLogin.mock.lastCall?.[1]).not.toHaveProperty("groupNote");
  });

  it("keeps a pending oauth url while clearing mailbox binding after the input diverges", async () => {
    vi.useFakeTimers();
    const updateOauthLogin = vi.fn().mockResolvedValue({
      loginId: "login-mailbox-bound",
      status: "pending",
      authUrl: "https://auth.openai.com/authorize?login=mailbox-bound",
      redirectUri: "http://localhost:1455/oauth/callback",
      expiresAt: "2026-03-13T10:00:00.000Z",
      accountId: null,
      error: null,
    });
    mockUpstreamAccounts({ updateOauthLogin });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=oauth",
      state: {
        draft: {
          oauth: {
            displayName: "Mailbox Bound",
            mailboxSession: {
              supported: true,
              sessionId: "mailbox-attached-9",
              emailAddress: "manual-existing@mail-tw.707079.xyz",
              expiresAt: "2026-03-13T10:00:00.000Z",
              source: "attached",
            },
            mailboxInput: "manual-existing@mail-tw.707079.xyz",
            session: {
              loginId: "login-mailbox-bound",
              status: "pending",
              authUrl:
                "https://auth.openai.com/authorize?login=mailbox-bound",
              redirectUri: "http://localhost:1455/oauth/callback",
              expiresAt: "2026-03-13T10:00:00.000Z",
              accountId: null,
              error: null,
            },
            sessionHint: "OAuth URL ready",
          },
        },
      },
    });

    await flushAsync();
    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(false);

    setInputValue('input[name="oauthMailboxInput"]', "different@example.com");
    await flushSessionSyncDebounce();

    expect(findButton(/Copy OAuth URL/i)?.disabled).toBe(false);
    expect(host?.textContent).not.toContain(
      "Generate a fresh OAuth URL before completing login.",
    );
    expect(host?.textContent).not.toContain("Attached mailbox");
    expect(updateOauthLogin).toHaveBeenCalledWith("login-mailbox-bound", {
      displayName: "Mailbox Bound",
      groupName: "",
      note: "",
      tagIds: [],
      isMother: false,
      mailboxSessionId: "",
      mailboxAddress: "",
    });
    expect(updateOauthLogin.mock.lastCall?.[1]).not.toHaveProperty("groupNote");
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
    mockUpstreamAccounts({
      beginOauthLogin,
      beginOauthMailboxSessionForAddress,
    });
    render("/account-pool/upstream-accounts/new?mode=oauth");

    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    setInputValue(
      'input[name="oauthMailboxInput"]',
      "manual-existing@example.com",
    );
    await flushAsync();
    clickButton(/Use address/i);
    await flushAsync();

    expect(beginOauthMailboxSessionForAddress).toHaveBeenCalledWith(
      "manual-existing@example.com",
    );
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
    mockUpstreamAccounts({
      beginOauthLogin,
      beginOauthMailboxSessionForAddress,
    });
    render("/account-pool/upstream-accounts/new?mode=oauth");

    setInputValue(
      'input[name="oauthMailboxInput"]',
      "manual-existing@mail-tw.707079.xyz",
    );
    await flushAsync();
    clickButton(/Use address/i);
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    expect(beginOauthMailboxSessionForAddress).toHaveBeenCalledWith(
      "manual-existing@mail-tw.707079.xyz",
    );
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

  it("auto-creates a supported manual mailbox address when moemail is missing it", async () => {
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
      sessionId: "mailbox-generated-9",
      emailAddress: "finance.lab.d5r@mail-tw.707079.xyz",
      expiresAt: "2026-03-13T10:00:00.000Z",
      source: "generated",
    });
    mockUpstreamAccounts({
      beginOauthLogin,
      beginOauthMailboxSessionForAddress,
    });
    render("/account-pool/upstream-accounts/new?mode=oauth");

    setInputValue(
      'input[name="oauthMailboxInput"]',
      "finance.lab.d5r@mail-tw.707079.xyz",
    );
    await flushAsync();
    clickButton(/Use address/i);
    await flushAsync();
    clickButton(/Generate OAuth URL/i);
    await flushAsync();

    expect(beginOauthMailboxSessionForAddress).toHaveBeenCalledWith(
      "finance.lab.d5r@mail-tw.707079.xyz",
    );
    expect(beginOauthLogin).toHaveBeenCalledWith({
      displayName: "finance.lab.d5r@mail-tw.707079.xyz",
      groupName: undefined,
      note: undefined,
      groupNote: undefined,
      accountId: undefined,
      tagIds: [],
      isMother: false,
      mailboxSessionId: "mailbox-generated-9",
      mailboxAddress: "finance.lab.d5r@mail-tw.707079.xyz",
    });
    expect(host?.textContent).toContain("finance.lab.d5r@mail-tw.707079.xyz");
    expect(host?.textContent).toContain("Generated mailbox");
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

    setInputValue(
      'input[name="oauthMailboxInput"]',
      "manual-existing@mail-tw.707079.xyz",
    );
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
    setInputValue(
      'input[name="oauthMailboxInput"]',
      "manual-existing@example.com",
    );
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
    setInputValue(
      'input[name="oauthMailboxInput"]',
      "MANUAL-EXISTING@MAIL-TW.707079.XYZ",
    );
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

    setInputValue(
      'input[name="oauthMailboxInput"]',
      "manual-existing@example.com",
    );
    await flushAsync();
    clickButton(/Use address/i);
    await flushAsync();

    expect(beginOauthMailboxSessionForAddress).toHaveBeenCalledWith(
      "manual-existing@example.com",
    );
    expect(host?.textContent).toContain(
      "This mailbox is not readable through the current MoeMail integration, so mailbox enhancements stay disabled.",
    );
    expect(findButton(/Generate OAuth URL/i)?.disabled).toBe(false);
    expect(findButton(/Copy code/i)?.disabled).toBe(true);
  });

  it("shows an explicit expired mailbox warning for single oauth", async () => {
    const expiredAt = new Date(Date.now() - 60_000).toISOString();
    mockUpstreamAccounts();
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
    const expiresAt = new Date(Date.now() + 60 * 60_000).toISOString();
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
    expect(host?.textContent).toContain("Check failed");
  });

  it("shows a checking badge while the mailbox refresh is in flight", async () => {
    const expiresAt = new Date(Date.now() + 60 * 60_000).toISOString();
    let releaseRefresh:
      | ((value: Array<Record<string, unknown>>) => void)
      | null = null;
    const getOauthMailboxStatuses = vi.fn().mockImplementation(
      () =>
        new Promise((resolve) => {
          releaseRefresh = resolve as typeof releaseRefresh;
        }),
    );
    mockUpstreamAccounts({ getOauthMailboxStatuses });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      state: {
        draft: {
          oauth: {
            displayName: "Checking Mailbox",
            mailboxSession: {
              supported: true,
              sessionId: "mailbox-checking",
              emailAddress: "checking@example.com",
              expiresAt,
              source: "generated",
            },
            mailboxInput: "checking@example.com",
          },
        },
      },
    });

    await act(async () => {
      await Promise.resolve();
    });

    expect(host?.textContent).toContain("Checking");

    await act(async () => {
      releaseRefresh?.([
        {
          sessionId: "mailbox-checking",
          emailAddress: "checking@example.com",
          expiresAt,
          latestCode: null,
          invite: null,
          invited: false,
          error: null,
        },
      ]);
      await Promise.resolve();
    });
  });

  it("auto refreshes the single mailbox status and shows received timestamps", async () => {
    vi.useFakeTimers();
    const expiresAt = new Date(Date.now() + 60 * 60_000).toISOString();
    const getOauthMailboxStatuses = vi
      .fn()
      .mockResolvedValueOnce([
        {
          sessionId: "mailbox-refresh-live",
          emailAddress: "live@example.com",
          expiresAt,
          latestCode: {
            value: "111111",
            source: "subject",
            updatedAt: "2026-03-13T10:00:00.000Z",
          },
          invite: {
            subject: "Workspace invite",
            copyValue: "https://example.com/invite",
            copyLabel: "invite-link",
            updatedAt: "2026-03-13T10:00:01.000Z",
          },
          invited: true,
          error: null,
        },
      ])
      .mockResolvedValueOnce([
        {
          sessionId: "mailbox-refresh-live",
          emailAddress: "live@example.com",
          expiresAt,
          latestCode: {
            value: "222222",
            source: "subject",
            updatedAt: "2026-03-13T10:00:05.000Z",
          },
          invite: {
            subject: "Workspace invite",
            copyValue: "https://example.com/invite",
            copyLabel: "invite-link",
            updatedAt: "2026-03-13T10:00:01.000Z",
          },
          invited: true,
          error: null,
        },
      ]);
    mockUpstreamAccounts({ getOauthMailboxStatuses });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=oauth",
      state: {
        draft: {
          oauth: {
            displayName: "Live Mailbox",
            mailboxSession: {
              supported: true,
              sessionId: "mailbox-refresh-live",
              emailAddress: "live@example.com",
              expiresAt,
              source: "generated",
            },
            mailboxInput: "live@example.com",
          },
        },
      },
    });

    await flushAsync();

    expect(getOauthMailboxStatuses).toHaveBeenCalledTimes(1);
    expect(host?.textContent).toContain("111111");
    expect(host?.textContent).toContain("Received at");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    await flushAsync();

    expect(getOauthMailboxStatuses).toHaveBeenCalledTimes(2);
    expect(host?.textContent).toContain("222222");
  });

  it("fetches mailbox status on demand for batch oauth flows", async () => {
    act(() => {
      root?.unmount();
    });
    host?.remove();
    host = null;
    root = null;

    const expiresAt = new Date(Date.now() + 60 * 60_000).toISOString();
    const batchStatuses = vi.fn().mockResolvedValue([
      {
        sessionId: "mailbox-manual-batch",
        emailAddress: "batch@example.com",
        expiresAt,
        latestCode: {
          value: "444444",
          source: "subject",
          updatedAt: "2026-03-13T10:00:00.000Z",
        },
        invite: null,
        invited: false,
        error: null,
      },
    ]);
    mockUpstreamAccounts({ getOauthMailboxStatuses: batchStatuses });
    render({
      pathname: "/account-pool/upstream-accounts/new",
      search: "?mode=batchOauth",
      state: {
        draft: {
          batchOauth: {
            rows: [
              {
                id: "row-1",
                displayName: "Row One",
                mailboxSession: {
                  supported: true,
                  sessionId: "mailbox-manual-batch",
                  emailAddress: "batch@example.com",
                  expiresAt,
                  source: "generated",
                },
                mailboxInput: "batch@example.com",
              },
            ],
          },
        },
      },
    });

    await flushAsync();
    expect(batchStatuses).toHaveBeenCalledTimes(1);
    expect(document.body.textContent ?? "").toContain("444444");
    expect(document.body.textContent ?? "").toMatch(/\d+s/);

    const row = getBatchRows()[0];
    const fetchButton = Array.from(
      row?.querySelectorAll<HTMLButtonElement>("button") ?? [],
    ).find((candidate) =>
      /Fetch/i.test(
        candidate.getAttribute("aria-label") ?? candidate.textContent ?? "",
      ),
    );
    expect(fetchButton).toBeInstanceOf(HTMLButtonElement);

    act(() => {
      fetchButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushAsync();

    expect(batchStatuses).toHaveBeenCalledTimes(2);
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
    setInputValue(
      'input[name="apiKeyUpstreamBaseUrl"]',
      "proxy.example.com/gateway",
    );

    expect(document.body.textContent).toContain("Use an absolute http(s) URL");
    expect(findButton(/Create API Key account/i)?.disabled).toBe(true);
    expect(createApiKeyAccount).not.toHaveBeenCalled();
  });
});

describe("UpstreamAccountCreatePage imported oauth", () => {
  function createImportedOauthFixture(index: number) {
    const email = `mailbox-${index}@duckmail.sbs`;
    const fileName = `${email}.json`;
    const content = JSON.stringify({
      type: "codex",
      email,
      account_id: `acct_${index}`,
      expired: "2026-03-20T00:00:00.000Z",
      access_token: "access",
      refresh_token: "refresh",
      id_token: "header.payload.signature",
    });
    const file = new File([content], fileName, {
      type: "application/json",
      lastModified: index,
    });
    return {
      file,
      content,
      fileName,
      email,
      chatgptAccountId: `acct_${index}`,
    };
  }

  function getImportedOauthSourceId(
    fixture: { file: File },
    selectionIndex = 0,
  ) {
    return `${fixture.file.name}:${fixture.file.size}:${fixture.file.lastModified}:${selectionIndex}`;
  }

  function buildImportedOauthRow(
    sourceId: string,
    fileName: string,
    email: string,
    accountId: string,
    status:
      | "ok"
      | "ok_exhausted"
      | "invalid"
      | "error"
      | "duplicate_in_input" = "ok",
    detail: string | null = null,
  ) {
    return {
      sourceId,
      fileName,
      email,
      chatgptAccountId: accountId,
      displayName: email,
      tokenExpiresAt: "2026-03-20T00:00:00.000Z",
      matchedAccount: null,
      status,
      detail,
      attempts: 1,
    };
  }

  function getImportedOauthResultRowsText() {
    return Array.from(document.querySelectorAll("tbody tr")).map(
      (row) => row.textContent ?? "",
    );
  }

  function buildPendingImportedOauthSnapshot(
    items: Array<{ sourceId: string; fileName: string }>,
  ): ImportedOauthValidationResponse {
    return {
      inputFiles: items.length,
      uniqueInInput: items.length,
      duplicateInInput: 0,
      rows: items.map((item) => ({
        sourceId: item.sourceId,
        fileName: item.fileName,
        email: null,
        chatgptAccountId: null,
        displayName: null,
        tokenExpiresAt: null,
        matchedAccount: null,
        status: "pending" as const,
        detail: null,
        attempts: 0,
      })),
    };
  }

  function installImportedOauthValidationJobFlow(options: {
    jobId?: string;
    rowsBySourceId?: Record<string, ReturnType<typeof buildImportedOauthRow>>;
    finalEvent?: "completed" | "failed" | "cancelled";
    finalError?: string;
    stepwise?: boolean;
  } = {}) {
    const {
      jobId = "job-1",
      rowsBySourceId = {},
      finalEvent = "completed",
      finalError = "Validation job failed",
      stepwise = false,
    } = options;
    let controller: {
      source: MockValidationEventSource;
      pendingSnapshot: ReturnType<typeof buildPendingImportedOauthSnapshot>;
      currentRows: ReturnType<typeof buildPendingImportedOauthSnapshot>["rows"];
      emitSnapshot: () => void;
      emitRow: (sourceId: string) => void;
      complete: () => void;
      fail: (error?: string) => void;
      cancel: () => void;
    } | null = null;

    const startImportedOauthValidationJob = vi.fn().mockImplementation(
      async ({
        items,
      }: {
        items: Array<{ sourceId: string; fileName: string }>;
      }) => {
        const pendingSnapshot = buildPendingImportedOauthSnapshot(items);
        const source = new MockValidationEventSource();
        const currentRows: ImportedOauthValidationRow[] = [...pendingSnapshot.rows];
        controller = {
          source,
          pendingSnapshot,
          currentRows,
          emitSnapshot() {
            source.emit("snapshot", {
              snapshot: pendingSnapshot,
              counts: buildImportedOauthValidationCounts(currentRows),
            });
          },
          emitRow(sourceId: string) {
            const nextRow = rowsBySourceId[sourceId];
            if (!nextRow) {
              throw new Error(`missing validation row for ${sourceId}`);
            }
            const index = currentRows.findIndex((row) => row.sourceId === sourceId);
            if (index >= 0) {
              currentRows[index] = {
                ...nextRow,
              };
            }
            source.emit("row", {
              row: nextRow,
              counts: buildImportedOauthValidationCounts(currentRows),
            });
          },
          complete() {
            source.emit("completed", {
              snapshot: {
                inputFiles: pendingSnapshot.inputFiles,
                uniqueInInput: currentRows.length,
                duplicateInInput: currentRows.filter(
                  (row) => row.status === "duplicate_in_input",
                ).length,
                rows: currentRows,
              },
              counts: buildImportedOauthValidationCounts(currentRows),
            });
          },
          fail(error = finalError) {
            source.emit("failed", {
              snapshot: {
                inputFiles: pendingSnapshot.inputFiles,
                uniqueInInput: currentRows.length,
                duplicateInInput: currentRows.filter(
                  (row) => row.status === "duplicate_in_input",
                ).length,
                rows: currentRows,
              },
              counts: buildImportedOauthValidationCounts(currentRows),
              error,
            });
          },
          cancel() {
            source.emit("cancelled", {
              snapshot: {
                inputFiles: pendingSnapshot.inputFiles,
                uniqueInInput: currentRows.length,
                duplicateInInput: currentRows.filter(
                  (row) => row.status === "duplicate_in_input",
                ).length,
                rows: currentRows,
              },
              counts: buildImportedOauthValidationCounts(currentRows),
            });
          },
        };

        apiMocks.createImportedOauthValidationJobEventSource.mockReturnValue(
          source as unknown as EventSource,
        );

        if (!stepwise) {
          window.setTimeout(() => {
            controller?.emitSnapshot();
            items.forEach((item) => controller?.emitRow(item.sourceId));
            if (finalEvent === "failed") {
              controller?.fail();
            } else if (finalEvent === "cancelled") {
              controller?.cancel();
            } else {
              controller?.complete();
            }
          }, 0);
        }

        return {
          jobId,
          snapshot: pendingSnapshot,
        };
      },
    );
    const stopImportedOauthValidationJob = vi.fn().mockResolvedValue(undefined);

    mockUpstreamAccounts({
      startImportedOauthValidationJob,
      stopImportedOauthValidationJob,
    });

    return {
      startImportedOauthValidationJob,
      stopImportedOauthValidationJob,
      getController: () => {
        if (!controller) {
          throw new Error("validation controller not initialized");
        }
        return controller;
      },
    };
  }

  it("opens import mode from the query string and validates selected files", async () => {
    const fixture = createImportedOauthFixture(1);
    const sourceId = getImportedOauthSourceId(fixture);
    const { startImportedOauthValidationJob } =
      installImportedOauthValidationJobFlow({
        rowsBySourceId: {
          [sourceId]: buildImportedOauthRow(
            sourceId,
            fixture.fileName,
            fixture.email,
            fixture.chatgptAccountId,
          ),
        },
      });
    mockUpstreamAccounts({
      startImportedOauthValidationJob,
    });
    render("/account-pool/upstream-accounts/new?mode=import");

    expect(host?.textContent).toContain("Import Codex OAuth JSON");

    const fileInput = host?.querySelector('input[name="importOauthFiles"]');
    if (!(fileInput instanceof HTMLInputElement)) {
      throw new Error("missing import file input");
    }

    await setFileInputFiles(fileInput, [fixture.file]);
    await flushAsync();

    clickButton(/validate and review/i);
    await flushAsync();
    await flushTimers();
    await flushAsync();

    expect(startImportedOauthValidationJob).toHaveBeenCalledWith({
      items: [
        expect.objectContaining({
          fileName: fixture.fileName,
          sourceId,
          content: fixture.content,
        }),
      ],
    });
    expect(document.body.textContent).toContain("Import validation");
    expect(document.body.textContent).toContain(fixture.fileName);
  });

  it("removes imported rows from the validation list without navigating away", async () => {
    const importedFixture = createImportedOauthFixture(1);
    const pendingFixture = createImportedOauthFixture(2);
    const importedSourceId = getImportedOauthSourceId(importedFixture, 0);
    const pendingSourceId = getImportedOauthSourceId(pendingFixture, 1);
    const { startImportedOauthValidationJob } =
      installImportedOauthValidationJobFlow({
        rowsBySourceId: {
          [importedSourceId]: buildImportedOauthRow(
            importedSourceId,
            importedFixture.fileName,
            importedFixture.email,
            importedFixture.chatgptAccountId,
          ),
          [pendingSourceId]: buildImportedOauthRow(
            pendingSourceId,
            pendingFixture.fileName,
            pendingFixture.email,
            pendingFixture.chatgptAccountId,
            "invalid",
            "Broken credential",
          ),
        },
      });
    const importOauthAccounts = vi.fn().mockResolvedValue({
      summary: {
        inputFiles: 2,
        selectedFiles: 1,
        created: 1,
        updatedExisting: 0,
        failed: 0,
      },
      results: [
        {
          sourceId: importedSourceId,
          fileName: importedFixture.fileName,
          email: importedFixture.email,
          chatgptAccountId: importedFixture.chatgptAccountId,
          accountId: 77,
          status: "created",
          detail: null,
          matchedAccount: null,
        },
      ],
    });
    mockUpstreamAccounts({
      startImportedOauthValidationJob,
      importOauthAccounts,
    });
    render("/account-pool/upstream-accounts/new?mode=import");

    const fileInput = host?.querySelector('input[name="importOauthFiles"]');
    if (!(fileInput instanceof HTMLInputElement)) {
      throw new Error("missing import file input");
    }

    await setFileInputFiles(fileInput, [
      importedFixture.file,
      pendingFixture.file,
    ]);
    await flushAsync();

    clickButton(/validate and review/i);
    await flushAsync();
    await flushTimers();
    await flushAsync();
    await flushAsync();

    clickBodyButton(/import usable files/i);
    await flushAsync();

    const importPayload = importOauthAccounts.mock.calls[0]?.[0];
    expect(importPayload).toBeDefined();
    expect(importPayload.items).toHaveLength(1);
    expect(importPayload.selectedSourceIds).toEqual([importedSourceId]);
    expect(importPayload.groupName).toBeUndefined();
    expect(importPayload.groupNote).toBeUndefined();
    expect(importPayload.tagIds).toEqual([]);

    expect(document.body.textContent).not.toContain(importedFixture.fileName);
    expect(document.body.textContent).toContain(pendingFixture.fileName);
    expect(document.body.textContent).toContain("Broken credential");
    expect(navigateMock).not.toHaveBeenCalled();
  });

  it("paginates validation results after every 100 rows", async () => {
    const fixtures = Array.from({ length: 130 }, (_, index) =>
      createImportedOauthFixture(index + 1),
    );
    const { startImportedOauthValidationJob } =
      installImportedOauthValidationJobFlow({
        rowsBySourceId: Object.fromEntries(
          fixtures.map((fixture, index) => {
            const sourceId = getImportedOauthSourceId(fixture, index);
            return [
              sourceId,
              buildImportedOauthRow(
                sourceId,
                fixture.fileName,
                fixture.email,
                fixture.chatgptAccountId,
              ),
            ];
          }),
        ),
      });
    mockUpstreamAccounts({ startImportedOauthValidationJob });
    render("/account-pool/upstream-accounts/new?mode=import");

    const fileInput = host?.querySelector('input[name="importOauthFiles"]');
    if (!(fileInput instanceof HTMLInputElement)) {
      throw new Error("missing import file input");
    }

    await setFileInputFiles(
      fileInput,
      fixtures.map((fixture) => fixture.file),
    );
    await flushAsync();

    clickButton(/validate and review/i);
    await flushAsync();
    await flushTimers();
    await flushAsync();

    const pageOneRows = getImportedOauthResultRowsText().join("\n");
    expect(startImportedOauthValidationJob).toHaveBeenCalledTimes(1);
    expect(startImportedOauthValidationJob.mock.calls[0]?.[0]?.items).toHaveLength(
      130,
    );
    expect(pageOneRows).toContain("mailbox-1@duckmail.sbs.json");
    expect(pageOneRows).toContain("mailbox-100@duckmail.sbs.json");
    expect(pageOneRows).not.toContain("mailbox-101@duckmail.sbs.json");
    expect(document.body.textContent).toContain("Page 1 / 2");

    clickBodyButton(/^next$/i);
    await flushAsync();

    const pageTwoRows = getImportedOauthResultRowsText().join("\n");
    expect(pageTwoRows).toContain("mailbox-101@duckmail.sbs.json");
    expect(pageTwoRows).toContain("mailbox-130@duckmail.sbs.json");
    expect(pageTwoRows).not.toContain("mailbox-1@duckmail.sbs.json");
    expect(document.body.textContent).toContain("Page 2 / 2");
  });

  it("updates validation progress one row at a time through SSE events", async () => {
    const fixtures = Array.from({ length: 3 }, (_, index) =>
      createImportedOauthFixture(index + 1),
    );
    const rowsBySourceId = Object.fromEntries(
      fixtures.map((fixture, index) => {
        const sourceId = getImportedOauthSourceId(fixture, index);
        return [
          sourceId,
          buildImportedOauthRow(
            sourceId,
            fixture.fileName,
            fixture.email,
            fixture.chatgptAccountId,
          ),
        ];
      }),
    );
    const { startImportedOauthValidationJob, getController } =
      installImportedOauthValidationJobFlow({
        rowsBySourceId,
        stepwise: true,
      });
    mockUpstreamAccounts({ startImportedOauthValidationJob });
    render("/account-pool/upstream-accounts/new?mode=import");

    const fileInput = host?.querySelector('input[name="importOauthFiles"]');
    if (!(fileInput instanceof HTMLInputElement)) {
      throw new Error("missing import file input");
    }

    await setFileInputFiles(
      fileInput,
      fixtures.map((fixture) => fixture.file),
    );
    await flushAsync();

    clickButton(/validate and review/i);
    await flushAsync();
    await flushTimers();
    await flushAsync();

    const controller = getController();
    controller.emitSnapshot();
    await flushAsync();
    expect(document.body.textContent).toContain("Checked 0 of 3");

    controller.emitRow(getImportedOauthSourceId(fixtures[0]!, 0));
    await flushAsync();
    expect(document.body.textContent).toContain("Checked 1 of 3");

    controller.emitRow(getImportedOauthSourceId(fixtures[1]!, 1));
    await flushAsync();
    expect(document.body.textContent).toContain("Checked 2 of 3");

    controller.emitRow(getImportedOauthSourceId(fixtures[2]!, 2));
    controller.complete();
    await flushAsync();
    expect(document.body.textContent).toContain("Checked 3 of 3");
  });

  it("closes the dialog and cancels the active validation job while checking", async () => {
    const fixture = createImportedOauthFixture(1);
    const sourceId = getImportedOauthSourceId(fixture);
    const { startImportedOauthValidationJob, stopImportedOauthValidationJob, getController } =
      installImportedOauthValidationJobFlow({
        rowsBySourceId: {
          [sourceId]: buildImportedOauthRow(
            sourceId,
            fixture.fileName,
            fixture.email,
            fixture.chatgptAccountId,
          ),
        },
        stepwise: true,
      });
    mockUpstreamAccounts({
      startImportedOauthValidationJob,
      stopImportedOauthValidationJob,
    });
    render("/account-pool/upstream-accounts/new?mode=import");

    const fileInput = host?.querySelector('input[name="importOauthFiles"]');
    if (!(fileInput instanceof HTMLInputElement)) {
      throw new Error("missing import file input");
    }

    await setFileInputFiles(fileInput, [fixture.file]);
    await flushAsync();

    clickButton(/validate and review/i);
    await flushAsync();

    getController().emitSnapshot();
    await flushAsync();
    expect(document.body.textContent).toContain("Import validation");

    clickBodyButton(/^close$/i);
    await flushAsync();

    expect(stopImportedOauthValidationJob).toHaveBeenCalledWith("job-1");
    expect(document.body.textContent).not.toContain("Import validation");
  });

  it("imports validated oauth rows in batches of 100 and clears the dialog when all batches succeed", async () => {
    const fixtures = Array.from({ length: 130 }, (_, index) =>
      createImportedOauthFixture(index + 1),
    );
    const { startImportedOauthValidationJob } =
      installImportedOauthValidationJobFlow({
        rowsBySourceId: Object.fromEntries(
          fixtures.map((fixture, index) => {
            const sourceId = getImportedOauthSourceId(fixture, index);
            return [
              sourceId,
              buildImportedOauthRow(
                sourceId,
                fixture.fileName,
                fixture.email,
                fixture.chatgptAccountId,
              ),
            ];
          }),
        ),
      });
    const importOauthAccounts = vi
      .fn()
      .mockImplementation(async ({ items, selectedSourceIds }) => ({
        summary: {
          inputFiles: items.length,
          selectedFiles: selectedSourceIds.length,
          created: selectedSourceIds.length,
          updatedExisting: 0,
          failed: 0,
        },
        results: selectedSourceIds.map((sourceId: string) => {
          const item = items.find(
            (candidate: { sourceId: string }) =>
              candidate.sourceId === sourceId,
          );
          return {
            sourceId,
            fileName: item?.fileName ?? sourceId,
            email: item?.fileName?.replace(/\\.json$/i, "") ?? sourceId,
            chatgptAccountId: `acct_${sourceId}`,
            accountId: 1,
            status: "created",
            detail: null,
            matchedAccount: null,
          };
        }),
      }));
    mockUpstreamAccounts({ startImportedOauthValidationJob, importOauthAccounts });
    render("/account-pool/upstream-accounts/new?mode=import");

    const fileInput = host?.querySelector('input[name="importOauthFiles"]');
    if (!(fileInput instanceof HTMLInputElement)) {
      throw new Error("missing import file input");
    }

    await setFileInputFiles(
      fileInput,
      fixtures.map((fixture) => fixture.file),
    );
    await flushAsync();

    clickButton(/validate and review/i);
    await flushAsync();
    await flushTimers();
    await flushAsync();
    await flushAsync();

    clickBodyButton(/import usable files/i);
    await flushAsync();

    expect(importOauthAccounts).toHaveBeenCalledTimes(2);
    expect(importOauthAccounts.mock.calls[0]?.[0]?.items).toHaveLength(100);
    expect(
      importOauthAccounts.mock.calls[0]?.[0]?.selectedSourceIds,
    ).toHaveLength(100);
    expect(importOauthAccounts.mock.calls[0]?.[0]?.validationJobId).toBe("job-1");
    expect(importOauthAccounts.mock.calls[1]?.[0]?.items).toHaveLength(30);
    expect(
      importOauthAccounts.mock.calls[1]?.[0]?.selectedSourceIds,
    ).toHaveLength(30);
    expect(importOauthAccounts.mock.calls[1]?.[0]?.validationJobId).toBe("job-1");
    expect(document.body.textContent).not.toContain("Import validation");
    expect(navigateMock).not.toHaveBeenCalled();
  });

  it("keeps failed import batches in the list after earlier batches succeed", async () => {
    const fixtures = Array.from({ length: 130 }, (_, index) =>
      createImportedOauthFixture(index + 1),
    );
    const { startImportedOauthValidationJob } =
      installImportedOauthValidationJobFlow({
        rowsBySourceId: Object.fromEntries(
          fixtures.map((fixture, index) => {
            const sourceId = getImportedOauthSourceId(fixture, index);
            return [
              sourceId,
              buildImportedOauthRow(
                sourceId,
                fixture.fileName,
                fixture.email,
                fixture.chatgptAccountId,
              ),
            ];
          }),
        ),
      });
    const importOauthAccounts = vi
      .fn()
      .mockImplementationOnce(async ({ items, selectedSourceIds }) => ({
        summary: {
          inputFiles: items.length,
          selectedFiles: selectedSourceIds.length,
          created: selectedSourceIds.length,
          updatedExisting: 0,
          failed: 0,
        },
        results: selectedSourceIds.map((sourceId: string) => ({
          sourceId,
          fileName: `${sourceId}.json`,
          email: sourceId,
          chatgptAccountId: `acct_${sourceId}`,
          accountId: 1,
          status: "created",
          detail: null,
          matchedAccount: null,
        })),
      }))
      .mockRejectedValueOnce(new Error("Import batch exploded"));
    mockUpstreamAccounts({ startImportedOauthValidationJob, importOauthAccounts });
    render("/account-pool/upstream-accounts/new?mode=import");

    const fileInput = host?.querySelector('input[name="importOauthFiles"]');
    if (!(fileInput instanceof HTMLInputElement)) {
      throw new Error("missing import file input");
    }

    await setFileInputFiles(
      fileInput,
      fixtures.map((fixture) => fixture.file),
    );
    await flushAsync();

    clickButton(/validate and review/i);
    await flushAsync();
    await flushTimers();
    await flushAsync();
    await flushAsync();

    clickBodyButton(/import usable files/i);
    await flushAsync();

    expect(importOauthAccounts).toHaveBeenCalledTimes(2);
    expect(document.body.textContent).toContain("Import batch exploded");
    expect(document.body.textContent).not.toContain(
      "mailbox-1@duckmail.sbs.json",
    );
    expect(document.body.textContent).toContain(
      "mailbox-101@duckmail.sbs.json",
    );
    expect(document.body.textContent).not.toContain("Page 1 / 2");
  });
});
