/* eslint-disable @typescript-eslint/ban-ts-comment, @typescript-eslint/no-unused-vars */
// @ts-nocheck
import ts from "typescript";
import suite1 from "./UpstreamAccountCreate.batch-oauth-a.txt?raw";
import suite2 from "./UpstreamAccountCreate.batch-oauth-b.txt?raw";
import suite3 from "./UpstreamAccountCreate.display-name-a.txt?raw";
import suite4 from "./UpstreamAccountCreate.display-name-b.txt?raw";
import suite5 from "./UpstreamAccountCreate.oauth-mailbox.txt?raw";
import suite6 from "./UpstreamAccountCreate.api-key.txt?raw";
import suite7 from "./UpstreamAccountCreate.imported-oauth.txt?raw";

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
  useForwardProxyBindingNodes: vi.fn(),
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

vi.mock("../../hooks/useForwardProxyBindingNodes", () => ({
  useForwardProxyBindingNodes: hookMocks.useForwardProxyBindingNodes,
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
        : (((event: Event) => listener.handleEvent(event)) as EventListener);
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
        : (((event: Event) => listener.handleEvent(event)) as EventListener);
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

function buildImportedOauthValidationCounts(rows: Array<{ status: string }>) {
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

const TEST_REQUIRED_GROUP_NAME = "prod";
const TEST_REQUIRED_BOUND_PROXY_KEYS = ["__direct__"];
const TEST_FORWARD_PROXY_NODES = [
  {
    key: "__direct__",
    displayName: "Direct",
    protocolLabel: "DIRECT",
    source: "direct",
    penalized: false,
    selectable: true,
    last24h: [],
  },
];
const TEST_GROUP_SUMMARIES = [
  TEST_REQUIRED_GROUP_NAME,
  "alpha",
  "beta",
  "custom",
  "analytics",
  "latam",
  "ops",
  "restored-group",
  "staging",
].map((groupName) => ({
  groupName,
  note: `${groupName} note`,
  boundProxyKeys: [...TEST_REQUIRED_BOUND_PROXY_KEYS],
  nodeShuntEnabled: false,
}));

function expectedGroupSelection(
  groupName = TEST_REQUIRED_GROUP_NAME,
  options?: { includeConcurrencyLimit?: boolean },
) {
  const selection = {
    groupName,
    groupBoundProxyKeys: [...TEST_REQUIRED_BOUND_PROXY_KEYS],
    groupNodeShuntEnabled: false,
  };
  if (options?.includeConcurrencyLimit === false) {
    return selection;
  }
  return {
    ...selection,
    concurrencyLimit: 0,
  };
}

let host: HTMLDivElement | null = null;
let root: Root | null = null;
let dateNowSpy: ReturnType<typeof vi.spyOn> | null = null;
const FIXED_NOW_MS = Date.parse("2026-03-12T00:00:00.000Z");

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
  dateNowSpy = vi.spyOn(Date, "now").mockReturnValue(FIXED_NOW_MS);
  vi.mocked(window.localStorage.getItem).mockImplementation((key: string) =>
    key === "codex-vibe-monitor.locale" ? "en" : null,
  );
  upstreamAccountsEventMocks.emitUpstreamAccountsChanged.mockReset();
  apiMocks.createImportedOauthValidationJobEventSource.mockReset();
  apiMocks.updateOauthLoginSessionKeepalive.mockReset();
  apiMocks.updateOauthLoginSessionKeepalive.mockResolvedValue(undefined);
  hookMocks.useForwardProxyBindingNodes.mockReturnValue({
    nodes: [],
    error: null,
    isLoading: false,
    refresh: vi.fn(),
    catalogState: {
      kind: "ready-empty",
      freshness: "fresh",
      isPending: false,
      hasNodes: false,
    },
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
  apiMocks.fetchUpstreamAccountDetail.mockReset();
  apiMocks.createImportedOauthValidationJobEventSource.mockReset();
  apiMocks.updateOauthLoginSessionKeepalive.mockReset();
  dateNowSpy?.mockRestore();
  dateNowSpy = null;
  vi.useRealTimers();
  vi.clearAllMocks();
});

function render(
  initialEntry: RenderEntry = "/account-pool/upstream-accounts/new",
) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  rerender(initialEntry);
}

function rerender(
  initialEntry: RenderEntry = "/account-pool/upstream-accounts/new",
) {
  const normalizedEntry =
    typeof initialEntry === "string"
      ? (() => {
          const parsed = new URL(initialEntry, "http://storybook.local");
          return {
            pathname: parsed.pathname,
            search: parsed.search,
            state: undefined,
          };
        })()
      : initialEntry;
  const baseState =
    normalizedEntry.state &&
    typeof normalizedEntry.state === "object" &&
    !Array.isArray(normalizedEntry.state)
      ? (normalizedEntry.state as Record<string, unknown>)
      : {};
  const baseDraft =
    baseState.draft &&
    typeof baseState.draft === "object" &&
    !Array.isArray(baseState.draft)
      ? (baseState.draft as Record<string, unknown>)
      : {};
  const oauthDraft =
    baseDraft.oauth &&
    typeof baseDraft.oauth === "object" &&
    !Array.isArray(baseDraft.oauth)
      ? (baseDraft.oauth as Record<string, unknown>)
      : {};
  const batchOauthDraft =
    baseDraft.batchOauth &&
    typeof baseDraft.batchOauth === "object" &&
    !Array.isArray(baseDraft.batchOauth)
      ? (baseDraft.batchOauth as Record<string, unknown>)
      : {};
  const importDraft =
    baseDraft.import &&
    typeof baseDraft.import === "object" &&
    !Array.isArray(baseDraft.import)
      ? (baseDraft.import as Record<string, unknown>)
      : {};
  const apiKeyDraft =
    baseDraft.apiKey &&
    typeof baseDraft.apiKey === "object" &&
    !Array.isArray(baseDraft.apiKey)
      ? (baseDraft.apiKey as Record<string, unknown>)
      : {};
  const entryWithDefaults = {
    pathname: normalizedEntry.pathname,
    search: normalizedEntry.search,
    state: {
      ...baseState,
      draft: {
        ...baseDraft,
        oauth: {
          groupName: TEST_REQUIRED_GROUP_NAME,
          ...oauthDraft,
        },
        batchOauth: {
          defaultGroupName: TEST_REQUIRED_GROUP_NAME,
          ...batchOauthDraft,
        },
        import: {
          defaultGroupName: TEST_REQUIRED_GROUP_NAME,
          ...importDraft,
        },
        apiKey: {
          groupName: TEST_REQUIRED_GROUP_NAME,
          ...apiKeyDraft,
        },
      },
    },
  } satisfies RenderEntry;
  act(() => {
    root?.render(
      <I18nProvider>
        <SystemNotificationProvider>
          <MemoryRouter initialEntries={[entryWithDefaults]}>
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

async function pasteIntoField(input: HTMLTextAreaElement, text: string) {
  await act(async () => {
    const event = new Event("paste", {
      bubbles: true,
      cancelable: true,
    }) as Event & {
      clipboardData: {
        getData: (type: string) => string;
      };
    };
    Object.defineProperty(event, "clipboardData", {
      configurable: true,
      value: {
        getData: (type: string) =>
          type === "text/plain" || type === "text" ? text : "",
      },
    });
    input.dispatchEvent(event);
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
  const groups = Array.isArray(overrides.groups)
    ? overrides.groups.map((group) => ({
        ...group,
        boundProxyKeys: Array.isArray(group.boundProxyKeys)
          ? group.boundProxyKeys
          : [...TEST_REQUIRED_BOUND_PROXY_KEYS],
      }))
    : TEST_GROUP_SUMMARIES;
  const hookState = {
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
    groups,
    forwardProxyNodes: TEST_FORWARD_PROXY_NODES,
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
    saveAccount: vi
      .fn()
      .mockImplementation(
        async (accountId: number, payload: Record<string, unknown>) =>
          buildSavedAccountDetail({
            id: accountId,
            displayName:
              typeof payload.displayName === "string"
                ? payload.displayName
                : "Row One",
            groupName:
              typeof payload.groupName === "string"
                ? payload.groupName
                : "prod",
            note: typeof payload.note === "string" ? payload.note : null,
            isMother: payload.isMother === true,
            tags: Array.isArray(payload.tagIds)
              ? payload.tagIds.map((tagId) => buildTagSummary(Number(tagId)))
              : [],
          }),
      ),
    ...overrides,
  };
  hookMocks.useUpstreamAccounts.mockReturnValue(hookState);
  hookMocks.useForwardProxyBindingNodes.mockReturnValue({
    nodes: hookState.forwardProxyNodes ?? [],
    error: null,
    isLoading: false,
    refresh: vi.fn(),
    catalogState: {
      kind:
        Array.isArray(hookState.forwardProxyNodes) && hookState.forwardProxyNodes.length > 0
          ? "ready-with-data"
          : "ready-empty",
      freshness: "fresh",
      isPending: false,
      hasNodes:
        Array.isArray(hookState.forwardProxyNodes) && hookState.forwardProxyNodes.length > 0,
    },
  });
  return hookState;
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

const scope: Record<string, unknown> = {};
Object.assign(scope, {
  act,
  createRoot,
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
  MemoryRouter,
  Route,
  Routes,
  SystemNotificationProvider,
  I18nProvider,
  UpstreamAccountCreatePage,
  navigateMock,
  hookMocks,
  upstreamAccountsEventMocks,
  apiMocks,
  MockValidationEventSource,
  buildImportedOauthValidationCounts,
  TEST_REQUIRED_GROUP_NAME,
  TEST_REQUIRED_BOUND_PROXY_KEYS,
  TEST_FORWARD_PROXY_NODES,
  TEST_GROUP_SUMMARIES,
  expectedGroupSelection,
  FIXED_NOW_MS,
  render,
  rerender,
  flushAsync,
  flushTimers,
  flushSessionSyncDebounce,
  flushSessionSyncRetry,
  setFileInputFiles,
  pasteIntoField,
  setInputValue,
  setFieldValue,
  setBodyInputValue,
  clickButton,
  clickBodyButton,
  findButton,
  findBodyButton,
  getBatchRows,
  pageTextContent,
  setComboboxValue,
  mockUpstreamAccounts,
  blurField,
  buildCompletedBatchOauthRow,
});
Object.defineProperties(scope, {
  host: { get: () => host, set: (value) => { host = value as typeof host; } },
  root: { get: () => root, set: (value) => { root = value as typeof root; } },
  dateNowSpy: { get: () => dateNowSpy, set: (value) => { dateNowSpy = value as typeof dateNowSpy; } },
});
const evalChunk = (chunk: string) => {
  const { outputText } = ts.transpileModule(chunk, {
    compilerOptions: {
      jsx: ts.JsxEmit.ReactJSX,
      module: ts.ModuleKind.CommonJS,
      target: ts.ScriptTarget.ES2020,
    },
    fileName: "suite.tsx",
  });
  const code = outputText.replace(/^"use strict";\s*/, "");
  new Function("scope", `with (scope) { ${code} }`)(scope);
};
evalChunk(suite1);
evalChunk(suite2);
evalChunk(suite3);
evalChunk(suite4);
evalChunk(suite5);
evalChunk(suite6);
evalChunk(suite7);
