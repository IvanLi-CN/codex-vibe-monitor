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
import type { EffectiveRoutingRule, TagSummary } from "../../lib/api";

const navigateMock = vi.hoisted(() => vi.fn());
const hookMocks = vi.hoisted(() => ({
  useUpstreamAccounts: vi.fn(),
  useUpstreamStickyConversations: vi.fn(),
  usePoolTags: vi.fn(),
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

vi.mock("../../hooks/usePoolTags", () => ({
  usePoolTags: hookMocks.usePoolTags,
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
  Object.defineProperty(HTMLElement.prototype, "hasPointerCapture", {
    configurable: true,
    writable: true,
    value: vi.fn(() => false),
  });
  Object.defineProperty(HTMLElement.prototype, "setPointerCapture", {
    configurable: true,
    writable: true,
    value: vi.fn(),
  });
  Object.defineProperty(HTMLElement.prototype, "releasePointerCapture", {
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
  hookMocks.usePoolTags.mockReturnValue({
    items: defaultPoolTags,
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

function clickFirstRosterRow() {
  const row = document.body.querySelector('tbody tr[role="button"]')
  if (!(row instanceof HTMLTableRowElement)) {
    throw new Error("missing roster row")
  }
  act(() => {
    row.dispatchEvent(new MouseEvent("click", { bubbles: true }))
  })
  return row
}

function clickCheckboxByLabel(matcher: RegExp) {
  const checkbox = Array.from(document.body.querySelectorAll('input[type="checkbox"]')).find(
    (candidate) =>
      candidate instanceof HTMLInputElement &&
      matcher.test(candidate.getAttribute('aria-label') || ''),
  )
  if (!(checkbox instanceof HTMLInputElement)) {
    throw new Error(`missing checkbox: ${matcher}`)
  }
  act(() => {
    checkbox.click()
  })
  return checkbox
}

function clickCombobox(matcher: RegExp) {
  const trigger = Array.from(document.body.querySelectorAll('button[role="combobox"]')).find(
    (candidate) =>
      candidate instanceof HTMLButtonElement &&
      matcher.test(
        candidate.getAttribute("aria-label") ||
          candidate.textContent ||
          "",
      ),
  )
  if (!(trigger instanceof HTMLButtonElement)) {
    throw new Error(`missing combobox: ${matcher}`)
  }
  pressButton(trigger)
  return trigger
}

function clickCommandItem(matcher: RegExp) {
  const item = Array.from(document.body.querySelectorAll("[cmdk-item]")).find(
    (candidate) =>
      candidate instanceof HTMLElement &&
      matcher.test(candidate.textContent || ""),
  )
  if (!(item instanceof HTMLElement)) {
    throw new Error(`missing command item: ${matcher}`)
  }
  act(() => {
    item.dispatchEvent(new MouseEvent("click", { bubbles: true }))
  })
  return item
}

function clickSelectOption(matcher: RegExp) {
  const option = Array.from(document.body.querySelectorAll('[role="option"]')).find(
    (candidate) =>
      candidate instanceof HTMLElement &&
      matcher.test(candidate.textContent || ''),
  )
  if (!(option instanceof HTMLElement)) {
    throw new Error(`missing select option: ${matcher}`)
  }
  act(() => {
    option.dispatchEvent(new MouseEvent("click", { bubbles: true }))
  })
  return option
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

const defaultEffectiveRoutingRule: EffectiveRoutingRule = {
  guardEnabled: false,
  lookbackHours: null,
  maxConversations: null,
  allowCutOut: true,
  allowCutIn: true,
  sourceTagIds: [],
  sourceTagNames: [],
  guardRules: [],
}

const defaultPoolTags: TagSummary[] = [
  {
    id: 1,
    name: "vip",
    routingRule: defaultEffectiveRoutingRule,
    accountCount: 2,
    groupCount: 1,
    updatedAt: "2026-03-16T00:00:00.000Z",
  },
  {
    id: 2,
    name: "burst-safe",
    routingRule: defaultEffectiveRoutingRule,
    accountCount: 1,
    groupCount: 1,
    updatedAt: "2026-03-16T00:00:00.000Z",
  },
  {
    id: 3,
    name: "prod-apac",
    routingRule: defaultEffectiveRoutingRule,
    accountCount: 1,
    groupCount: 1,
    updatedAt: "2026-03-16T00:00:00.000Z",
  },
  {
    id: 4,
    name: "sticky-pool",
    routingRule: defaultEffectiveRoutingRule,
    accountCount: 1,
    groupCount: 1,
    updatedAt: "2026-03-16T00:00:00.000Z",
  },
]

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject }
}

function mockAccountsPage(options?: {
  saveRouting?: ReturnType<typeof vi.fn>
  routing?: {
    writesEnabled: boolean
    apiKeyConfigured: boolean
    maskedApiKey: string | null
    timeouts: {
      responsesFirstByteTimeoutSecs: number
      compactFirstByteTimeoutSecs: number
      responsesStreamTimeoutSecs: number
      compactStreamTimeoutSecs: number
    }
  } | null
  item?: Record<string, unknown>
  selectedSummary?: Record<string, unknown>
  detail?: Record<string, unknown>
}) {
  const saveRouting = options?.saveRouting ?? vi.fn();
  const compactSupport = {
    status: "unsupported" as const,
    observedAt: "2026-03-16T02:08:00.000Z",
    reason: "No available channel for compact model gpt-5.4-openai-compact",
  };
  const routingTimeouts = {
    responsesFirstByteTimeoutSecs: 120,
    compactFirstByteTimeoutSecs: 300,
    responsesStreamTimeoutSecs: 300,
    compactStreamTimeoutSecs: 300,
  };
  const primaryItem = {
    id: 5,
    kind: "oauth_codex",
    provider: "codex",
    displayName: "Existing OAuth",
    groupName: "prod",
    status: "active",
    displayStatus: "active",
    enabled: true,
    isMother: true,
    planType: "team",
    lastActionSource: "call",
    lastActionReasonCode: "upstream_http_429_quota_exhausted",
    lastActionHttpStatus: 429,
    lastActionAt: "2026-03-16T02:06:00.000Z",
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
    compactSupport,
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
    ...(options?.item ?? {}),
  };
  const selectedSummary = {
    ...primaryItem,
    lastSuccessfulSyncAt: "2026-03-16T01:55:00.000Z",
    lastActivityAt: "2026-03-16T02:05:00.000Z",
    lastAction: "route_hard_unavailable",
    lastActionSource: "call",
    lastActionReasonCode: "upstream_http_429_quota_exhausted",
    lastActionReasonMessage: "Weekly cap exhausted for this account",
    lastActionHttpStatus: 429,
    lastActionInvokeId: "invk_action_001",
    lastActionAt: "2026-03-16T02:06:00.000Z",
    ...(options?.selectedSummary ?? {}),
  };
  const detail = {
    ...selectedSummary,
    email: "dup@example.com",
    chatgptAccountId: "org_1",
    chatgptUserId: "user_1",
    history: [],
    recentActions: [
      {
        id: 71,
        occurredAt: "2026-03-16T02:06:00.000Z",
        action: "route_hard_unavailable",
        source: "call",
        reasonCode: "upstream_http_429_quota_exhausted",
        reasonMessage: "Weekly cap exhausted for this account",
        httpStatus: 429,
        failureKind: "upstream_http_429_quota_exhausted",
        invokeId: "invk_action_001",
        stickyKey: "sticky-dup-001",
        createdAt: "2026-03-16T02:06:00.000Z",
      },
    ],
    ...(options?.detail ?? {}),
  };
  hookMocks.useUpstreamAccounts.mockReturnValue({
    items: [
      selectedSummary,
      {
        id: 9,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Another OAuth",
        groupName: "prod",
        status: "active",
        displayStatus: "active",
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
    hasUngroupedAccounts: true,
    writesEnabled: true,
    total: 2,
    page: 1,
    pageSize: 20,
    metrics: {
      total: 2,
      oauth: 2,
      apiKey: 0,
      attention: 0,
    },
    selectedId: 5,
    selectedSummary,
    detail,
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
    beginOauthMailboxSession: vi.fn(),
    beginOauthMailboxSessionForAddress: vi.fn(),
    getOauthMailboxStatuses: vi.fn(),
    removeOauthMailboxSession: vi.fn(),
    getLoginSession: vi.fn(),
    completeOauthLogin: vi.fn(),
    createApiKeyAccount: vi.fn(),
    saveAccount: vi.fn(),
    saveRouting,
    saveGroupNote: vi.fn(),
    runBulkAction: vi.fn(),
    startBulkSyncJob: vi.fn(),
    getBulkSyncJob: vi.fn(),
    stopBulkSyncJob: vi.fn(),
    runSync: vi.fn(),
    removeAccount: vi.fn(),
    groups: [],
    routing:
      options && 'routing' in options
        ? options.routing
        : {
            writesEnabled: true,
            apiKeyConfigured: false,
            maskedApiKey: null,
            timeouts: routingTimeouts,
          },
  });
  return { saveRouting, compactSupport, routingTimeouts };
}

describe("UpstreamAccountsPage duplicates", () => {
  it("renders the compact roster header and folded metadata chips", () => {
    mockAccountsPage();
    render("/account-pool/upstream-accounts");

    const headerCells = Array.from(document.body.querySelectorAll("thead th")).map((cell) =>
      cell.textContent?.trim() || "",
    );
    expect(headerCells).toEqual(["", "Account", "Sync / Call", "Windows", ""]);
    expect(document.body.textContent).toContain("vip");
    expect(document.body.textContent).toContain("+1");
    expect(document.body.textContent).toContain("team");
  });

  it("shows action-first roster summaries and keeps the concrete failure message in hover text", () => {
    mockAccountsPage();
    render("/account-pool/upstream-accounts");

    const firstRow = document.body.querySelector('tbody tr[role="button"]');
    if (!(firstRow instanceof HTMLTableRowElement)) {
      throw new Error("missing roster row");
    }

    expect(firstRow.textContent).toContain("Hard unavailable");
    expect(firstRow.textContent).toContain("Upstream quota or weekly cap was exhausted");
    expect(firstRow.textContent).toContain("HTTP 429");
    expect(document.body.querySelector('[title*="Weekly cap exhausted for this account"]')).not.toBeNull();
  });

  it("shows latest account action details and recent events in the drawer", async () => {
    mockAccountsPage();
    render("/account-pool/upstream-accounts");

    clickFirstRosterRow();
    await flushAsync();

    expect(document.body.textContent).toContain("Latest account action");
    expect(document.body.textContent).toContain("Hard unavailable");
    expect(document.body.textContent).toContain("Weekly cap exhausted for this account");
    expect(document.body.textContent).toContain("Recent account events");
    expect(document.body.textContent).toContain("invk_action_001");
  });

  it("renders blocked recovery actions with translated source and reason labels", async () => {
    mockAccountsPage({
      selectedSummary: {
        status: "error",
        displayStatus: "error_other",
        healthStatus: "error_other",
        lastAction: "sync_recovery_blocked",
        lastActionSource: "sync_maintenance",
        lastActionReasonCode: "quota_still_exhausted",
        lastActionReasonMessage:
          "latest usage snapshot still shows an exhausted upstream usage limit window",
        lastActionHttpStatus: null,
      },
      detail: {
        status: "error",
        displayStatus: "error_other",
        healthStatus: "error_other",
        lastAction: "sync_recovery_blocked",
        lastActionSource: "sync_maintenance",
        lastActionReasonCode: "quota_still_exhausted",
        lastActionReasonMessage:
          "latest usage snapshot still shows an exhausted upstream usage limit window",
        lastActionHttpStatus: null,
        recentActions: [
          {
            id: 72,
            occurredAt: "2026-03-16T03:10:00.000Z",
            action: "sync_recovery_blocked",
            source: "sync_maintenance",
            reasonCode: "quota_still_exhausted",
            reasonMessage:
              "latest usage snapshot still shows an exhausted upstream usage limit window",
            httpStatus: null,
            failureKind: "upstream_http_429_quota_exhausted",
            invokeId: null,
            stickyKey: null,
            createdAt: "2026-03-16T03:10:00.000Z",
          },
        ],
      },
    });
    render("/account-pool/upstream-accounts");

    clickFirstRosterRow();
    await flushAsync();

    expect(document.body.textContent).toContain("Recovery still blocked");
    expect(document.body.textContent).toContain("Maintenance sync");
    expect(document.body.textContent).toContain(
      "Fresh usage snapshot still shows an exhausted limit window",
    );
  });

  it("shows compact support state and saves routing timeouts", async () => {
    const saveRouting = vi.fn().mockResolvedValue(undefined);
    const { compactSupport, routingTimeouts } = mockAccountsPage({ saveRouting });
    render("/account-pool/upstream-accounts");

    expect(document.body.textContent).toContain("Compact unsupported");

    clickFirstRosterRow();
    await flushAsync();

    expect(document.body.textContent).toContain("Compact support");
    expect(document.body.textContent).toContain("Unsupported");
    expect(document.body.textContent).toContain(compactSupport.reason);

    clickButton(/Edit routing settings/i);
    const compactInput = document.body.querySelector(
      'input[name="compactFirstByteTimeoutSecs"]',
    );
    expect(compactInput).toBeInstanceOf(HTMLInputElement);
    expect((compactInput as HTMLInputElement).value).toBe("300");

    setInputValue('input[name="compactFirstByteTimeoutSecs"]', "420");
    clickButton(/Save settings/i);
    await flushAsync();

    expect(saveRouting).toHaveBeenCalledWith({
      apiKey: undefined,
      timeouts: {
        ...routingTimeouts,
        compactFirstByteTimeoutSecs: 420,
      },
    });
  });

  it("keeps the routing card summary-only while the dialog still exposes advanced fields", async () => {
    mockAccountsPage({
      routing: {
        writesEnabled: true,
        apiKeyConfigured: true,
        maskedApiKey: "pool-live••••",
        timeouts: {
          responsesFirstByteTimeoutSecs: 120,
          compactFirstByteTimeoutSecs: 300,
          responsesStreamTimeoutSecs: 300,
          compactStreamTimeoutSecs: 300,
        },
      },
    });
    render("/account-pool/upstream-accounts");

    expect(document.body.textContent).toContain("Current pool API key");
    expect(document.body.textContent).toContain("pool-live••••");
    expect(document.body.textContent).not.toContain("Priority sync interval");
    expect(document.body.textContent).not.toContain("Secondary sync interval");
    expect(document.body.textContent).not.toContain("Priority available account cap");
    expect(document.body.textContent).not.toContain("Standard response first byte timeout");
    expect(document.body.textContent).not.toContain("Compact response first byte timeout");
    expect(document.body.textContent).not.toContain("Standard stream completion timeout");
    expect(document.body.textContent).not.toContain("Compact stream completion timeout");

    clickButton(/Edit routing settings/i);
    await flushAsync();

    expect(document.body.textContent).toContain("Priority sync interval");
    expect(document.body.textContent).toContain("Secondary sync interval");
    expect(document.body.textContent).toContain("Priority available account cap");
    expect(document.body.textContent).toContain("Standard response first byte timeout");
    expect(document.body.textContent).toContain("Compact response first byte timeout");
    expect(document.body.textContent).toContain("Standard stream completion timeout");
    expect(document.body.textContent).toContain("Compact stream completion timeout");
  });

  it("rejects non-integer routing timeout edits before saving", async () => {
    const saveRouting = vi.fn().mockResolvedValue(undefined);
    mockAccountsPage({ saveRouting });
    render("/account-pool/upstream-accounts");

    clickButton(/Edit routing settings/i);
    setInputValue('input[name="compactFirstByteTimeoutSecs"]', "1.5");
    clickButton(/Save settings/i);
    await flushAsync();

    expect(saveRouting).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain("must be a positive integer");
  });

  it("keeps routing save disabled until settings have loaded", async () => {
    mockAccountsPage({ routing: null });
    render("/account-pool/upstream-accounts");

    const editButton = Array.from(document.body.querySelectorAll("button")).find((button) =>
      /edit routing settings/i.test(button.textContent || ""),
    );
    expect(editButton).toBeInstanceOf(HTMLButtonElement);
    expect((editButton as HTMLButtonElement).disabled).toBe(true);
  });

  it("keeps routing settings inspectable in read-only mode", async () => {
    mockAccountsPage({
      routing: {
        writesEnabled: false,
        apiKeyConfigured: true,
        maskedApiKey: "pool-live••••",
        timeouts: {
          responsesFirstByteTimeoutSecs: 120,
          compactFirstByteTimeoutSecs: 300,
          responsesStreamTimeoutSecs: 300,
          compactStreamTimeoutSecs: 300,
        },
      },
    });
    render("/account-pool/upstream-accounts");

    const editButton = clickButton(/Edit routing settings/i);
    expect(editButton.disabled).toBe(false);
    await flushAsync();
    await flushTimers();

    const compactInput = document.body.querySelector(
      'input[name="compactFirstByteTimeoutSecs"]',
    );
    expect(compactInput).toBeInstanceOf(HTMLInputElement);
    expect((compactInput as HTMLInputElement).disabled).toBe(true);

    const apiKeyInput = document.body.querySelector('input[name="poolRoutingSecret"]');
    expect(apiKeyInput).toBeInstanceOf(HTMLInputElement);
    expect((apiKeyInput as HTMLInputElement).disabled).toBe(true);

    const generateButton = findButton(/Generate/i);
    if (!(generateButton instanceof HTMLButtonElement)) {
      throw new Error("missing generate button");
    }
    expect(generateButton.disabled).toBe(true);

    const saveButton = findButton(/Save settings/i);
    if (!(saveButton instanceof HTMLButtonElement)) {
      throw new Error("missing save button");
    }
    expect(saveButton.disabled).toBe(true);
  });

  it("preserves unsaved routing edits while the dialog is open during refresh", async () => {
    mockAccountsPage();
    render("/account-pool/upstream-accounts");

    clickButton(/Edit routing settings/i);
    setInputValue('input[name="compactFirstByteTimeoutSecs"]', "420");

    mockAccountsPage();
    rerender("/account-pool/upstream-accounts");
    await flushAsync();

    const compactInput = document.body.querySelector(
      'input[name="compactFirstByteTimeoutSecs"]',
    );
    expect(compactInput).toBeInstanceOf(HTMLInputElement);
    expect((compactInput as HTMLInputElement).value).toBe("420");
  });

  it("disables routing saves when the dialog becomes read-only during refresh", async () => {
    mockAccountsPage();
    render("/account-pool/upstream-accounts");

    clickButton(/Edit routing settings/i);
    setInputValue('input[name="compactFirstByteTimeoutSecs"]', "420");

    mockAccountsPage({
      routing: {
        writesEnabled: false,
        apiKeyConfigured: false,
        maskedApiKey: null,
        timeouts: {
          responsesFirstByteTimeoutSecs: 120,
          compactFirstByteTimeoutSecs: 300,
          responsesStreamTimeoutSecs: 300,
          compactStreamTimeoutSecs: 300,
        },
      },
    });
    rerender("/account-pool/upstream-accounts");
    await flushAsync();
    await flushTimers();

    const compactInput = document.body.querySelector(
      'input[name="compactFirstByteTimeoutSecs"]',
    );
    expect(compactInput).toBeInstanceOf(HTMLInputElement);
    expect((compactInput as HTMLInputElement).value).toBe("300");
    expect((compactInput as HTMLInputElement).disabled).toBe(true);

    const saveButton = findButton(/Save settings/i);
    if (!(saveButton instanceof HTMLButtonElement)) {
      throw new Error("missing save button");
    }
    expect(saveButton.disabled).toBe(true);
  });

  it("resyncs inspect-only routing drafts before writes are re-enabled", async () => {
    mockAccountsPage({
      routing: {
        writesEnabled: false,
        apiKeyConfigured: true,
        maskedApiKey: "pool-live••••",
        timeouts: {
          responsesFirstByteTimeoutSecs: 120,
          compactFirstByteTimeoutSecs: 300,
          responsesStreamTimeoutSecs: 300,
          compactStreamTimeoutSecs: 300,
        },
      },
    });
    render("/account-pool/upstream-accounts");

    clickButton(/Edit routing settings/i);
    await flushAsync();
    await flushTimers();

    mockAccountsPage({
      routing: {
        writesEnabled: true,
        apiKeyConfigured: true,
        maskedApiKey: "pool-next••••",
        timeouts: {
          responsesFirstByteTimeoutSecs: 150,
          compactFirstByteTimeoutSecs: 360,
          responsesStreamTimeoutSecs: 330,
          compactStreamTimeoutSecs: 360,
        },
      },
    });
    rerender("/account-pool/upstream-accounts");
    await flushAsync();
    await flushTimers();

    const compactInput = document.body.querySelector(
      'input[name="compactFirstByteTimeoutSecs"]',
    );
    expect(compactInput).toBeInstanceOf(HTMLInputElement);
    expect((compactInput as HTMLInputElement).value).toBe("360");
    expect((compactInput as HTMLInputElement).disabled).toBe(false);

    const saveButton = findButton(/Save settings/i);
    if (!(saveButton instanceof HTMLButtonElement)) {
      throw new Error("missing save button");
    }
    expect(saveButton.disabled).toBe(true);
  });

  it("closes the routing dialog when routing settings disappear during refresh", async () => {
    mockAccountsPage();
    render("/account-pool/upstream-accounts");

    clickButton(/Edit routing settings/i);
    await flushAsync();
    await flushTimers();

    expect(
      document.body.querySelector('input[name="compactFirstByteTimeoutSecs"]'),
    ).toBeInstanceOf(HTMLInputElement);

    mockAccountsPage({ routing: null });
    rerender("/account-pool/upstream-accounts");
    await flushAsync();
    await flushTimers();

    expect(
      document.body.querySelector('input[name="compactFirstByteTimeoutSecs"]'),
    ).toBeNull();
  });

  it("passes all-match tag filters to the roster hook", () => {
    mockAccountsPage();
    render("/account-pool/upstream-accounts");

    clickCombobox(/filter accounts by tags/i);
    clickCommandItem(/^vip$/i);
    clickCommandItem(/^burst-safe$/i);

    expect(hookMocks.useUpstreamAccounts).toHaveBeenLastCalledWith({
      groupSearch: undefined,
      groupUngrouped: undefined,
      workStatus: undefined,
      enableStatus: undefined,
      healthStatus: undefined,
      page: 1,
      pageSize: 20,
      tagIds: [1, 2],
    });
  });

  it("passes ungrouped and tag filters to the roster hook together", () => {
    mockAccountsPage();
    render("/account-pool/upstream-accounts");

    clickCombobox(/account groups/i);
    clickCommandItem(/^ungrouped$/i);
    clickCombobox(/filter accounts by tags/i);
    clickCommandItem(/^vip$/i);
    clickCommandItem(/^burst-safe$/i);

    expect(hookMocks.useUpstreamAccounts).toHaveBeenLastCalledWith({
      groupSearch: undefined,
      groupUngrouped: true,
      workStatus: undefined,
      enableStatus: undefined,
      healthStatus: undefined,
      page: 1,
      pageSize: 20,
      tagIds: [1, 2],
    });
  });

  it("passes split status filters to the roster hook", () => {
    mockAccountsPage();
    render("/account-pool/upstream-accounts");

    clickCombobox(/work status/i);
    clickSelectOption(/^rate limited$/i);
    clickCombobox(/enable status/i);
    clickSelectOption(/^enabled$/i);
    clickCombobox(/account health/i);
    clickSelectOption(/^needs re-auth$/i);

    expect(hookMocks.useUpstreamAccounts).toHaveBeenLastCalledWith({
      groupSearch: undefined,
      groupUngrouped: undefined,
      workStatus: "rate_limited",
      enableStatus: "enabled",
      healthStatus: "needs_reauth",
      page: 1,
      pageSize: 20,
      tagIds: undefined,
    });
  });

  it("shows duplicate warnings in the roster and detail drawer", () => {
    mockAccountsPage();
    render("/account-pool/upstream-accounts");

    expect(document.body.textContent).toContain("Duplicate");
    clickFirstRosterRow();
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

  it("prevents starting bulk sync twice while the create request is still pending", async () => {
    const startBulkSyncRequest = deferred<never>();
    const startBulkSyncJob = vi.fn(() => startBulkSyncRequest.promise);

    hookMocks.useUpstreamAccounts.mockReturnValue({
      items: [
        {
          id: 5,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Existing OAuth",
          groupName: "prod",
          status: "active",
          displayStatus: "active",
          enabled: true,
          isMother: true,
          planType: "team",
          primaryWindow: null,
          secondaryWindow: null,
          credits: null,
          localLimits: null,
          tags: [],
          effectiveRoutingRule: defaultEffectiveRoutingRule,
        },
      ],
      hasUngroupedAccounts: true,
      writesEnabled: true,
      total: 1,
      page: 1,
      pageSize: 20,
      metrics: {
        total: 1,
        oauth: 1,
        apiKey: 0,
        attention: 0,
      },
      selectedId: 5,
      selectedSummary: {
        id: 5,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Existing OAuth",
        groupName: "prod",
        status: "active",
        displayStatus: "active",
        enabled: true,
        isMother: true,
        planType: "team",
        primaryWindow: null,
        secondaryWindow: null,
        credits: null,
        localLimits: null,
        tags: [],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
      detail: null,
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
      beginOauthMailboxSession: vi.fn(),
      beginOauthMailboxSessionForAddress: vi.fn(),
      getOauthMailboxStatuses: vi.fn(),
      removeOauthMailboxSession: vi.fn(),
      getLoginSession: vi.fn(),
      completeOauthLogin: vi.fn(),
      createApiKeyAccount: vi.fn(),
      saveAccount: vi.fn(),
      saveRouting: vi.fn(),
      saveGroupNote: vi.fn(),
      runBulkAction: vi.fn(),
      startBulkSyncJob,
      getBulkSyncJob: vi.fn(),
      stopBulkSyncJob: vi.fn(),
      runSync: vi.fn(),
      removeAccount: vi.fn(),
      groups: [],
      routing: { apiKeyConfigured: false, maskedApiKey: null },
    });

    render("/account-pool/upstream-accounts");

    clickCheckboxByLabel(/select existing oauth/i);
    const syncButton = clickButton(/sync selected/i);
    await flushAsync();

    expect(startBulkSyncJob).toHaveBeenCalledTimes(1);
    expect(syncButton.disabled).toBe(true);

    act(() => {
      syncButton.click();
    });
    await flushAsync();

    expect(startBulkSyncJob).toHaveBeenCalledTimes(1);

    startBulkSyncRequest.reject(new Error("network interrupted"));
    await flushAsync();
  });

  it("prioritizes the selected accounts tag union when removing tags in bulk", () => {
    hookMocks.useUpstreamAccounts.mockReturnValue({
      items: [
        {
          id: 5,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Existing OAuth",
          groupName: "prod",
          status: "active",
          displayStatus: "active",
          enabled: true,
          isMother: true,
          planType: "team",
          primaryWindow: null,
          secondaryWindow: null,
          credits: null,
          localLimits: null,
          tags: [{ id: 1, name: "vip", routingRule: defaultEffectiveRoutingRule }],
          effectiveRoutingRule: defaultEffectiveRoutingRule,
        },
        {
          id: 9,
          kind: "oauth_codex",
          provider: "codex",
          displayName: "Another OAuth",
          groupName: "prod",
          status: "active",
          displayStatus: "active",
          enabled: true,
          isMother: false,
          planType: "pro",
          primaryWindow: null,
          secondaryWindow: null,
          credits: null,
          localLimits: null,
          tags: [{ id: 2, name: "burst-safe", routingRule: defaultEffectiveRoutingRule }],
          effectiveRoutingRule: defaultEffectiveRoutingRule,
        },
      ],
      hasUngroupedAccounts: true,
      writesEnabled: true,
      total: 2,
      page: 1,
      pageSize: 20,
      metrics: {
        total: 2,
        oauth: 2,
        apiKey: 0,
        attention: 0,
      },
      selectedId: 5,
      selectedSummary: {
        id: 5,
        kind: "oauth_codex",
        provider: "codex",
        displayName: "Existing OAuth",
        groupName: "prod",
        status: "active",
        displayStatus: "active",
        enabled: true,
        isMother: true,
        planType: "team",
        primaryWindow: null,
        secondaryWindow: null,
        credits: null,
        localLimits: null,
        tags: [{ id: 1, name: "vip", routingRule: defaultEffectiveRoutingRule }],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
      detail: null,
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
      beginOauthMailboxSession: vi.fn(),
      beginOauthMailboxSessionForAddress: vi.fn(),
      getOauthMailboxStatuses: vi.fn(),
      removeOauthMailboxSession: vi.fn(),
      getLoginSession: vi.fn(),
      completeOauthLogin: vi.fn(),
      createApiKeyAccount: vi.fn(),
      saveAccount: vi.fn(),
      saveRouting: vi.fn(),
      saveGroupNote: vi.fn(),
      runBulkAction: vi.fn(),
      startBulkSyncJob: vi.fn(),
      getBulkSyncJob: vi.fn(),
      stopBulkSyncJob: vi.fn(),
      runSync: vi.fn(),
      removeAccount: vi.fn(),
      groups: [],
      routing: { apiKeyConfigured: false, maskedApiKey: null },
    });

    render("/account-pool/upstream-accounts");

    clickCheckboxByLabel(/select existing oauth/i);
    clickCheckboxByLabel(/select another oauth/i);
    clickButton(/remove tags/i);

    const dialog = document.body.querySelector('[role="dialog"]');
    if (!(dialog instanceof HTMLElement)) {
      throw new Error("missing bulk remove tags dialog");
    }

    const combobox = dialog.querySelector('button[role="combobox"]');
    if (!(combobox instanceof HTMLButtonElement)) {
      throw new Error("missing bulk tag combobox");
    }
    pressButton(combobox);

    const options = Array.from(document.body.querySelectorAll('[cmdk-item]')) as HTMLElement[];
    expect(options.map((option) => option.textContent?.trim())).toEqual([
      "burst-safe",
      "vip",
      "prod-apac",
      "sticky-pool",
    ]);
    expect(options[0]?.getAttribute("aria-disabled")).toBe("false");
    expect(options[1]?.getAttribute("aria-disabled")).toBe("false");
    expect(options[2]?.getAttribute("aria-disabled")).toBe("true");
    expect(options[3]?.getAttribute("aria-disabled")).toBe("true");
  });

  it("blocks saving when the edited display name conflicts with another account", () => {
    mockAccountsPage();
    render("/account-pool/upstream-accounts");

    clickFirstRosterRow();
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

    clickButton(/Edit routing settings/i);
    setInputValue('input[name="secondarySyncIntervalSecs"]', "2400");
    clickButton(/Save settings/i);
    await flushAsync();

    expect(document.body.textContent).toContain("Routing failed");

    clickFirstRosterRow();
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

  it("shows routing and account errors at the same time", async () => {
    const runSync = vi.fn().mockRejectedValue(new Error("Sync failed"));
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

    clickButton(/Edit routing settings/i);
    setInputValue('input[name="secondarySyncIntervalSecs"]', "2400");
    clickButton(/Save settings/i);
    await flushAsync();
    clickFirstRosterRow();
    clickButton(/Sync now/i);
    await flushAsync();

    expect(document.body.textContent).toContain("Routing failed");
    expect(document.body.textContent).toContain("Sync failed");
  });

  it("saves maintenance settings without requiring a new pool key", async () => {
    const saveRouting = vi.fn().mockResolvedValue(undefined);

    hookMocks.useUpstreamAccounts.mockReturnValue({
      items: [],
      writesEnabled: false,
      selectedId: null,
      selectedSummary: null,
      detail: null,
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
      runSync: vi.fn(),
      removeAccount: vi.fn(),
      routing: {
        writesEnabled: true,
        apiKeyConfigured: true,
        maskedApiKey: "pool-live••••",
        maintenance: {
          primarySyncIntervalSecs: 300,
          secondarySyncIntervalSecs: 1800,
          priorityAvailableAccountCap: 100,
        },
      },
      groups: [],
    });
    hookMocks.useUpstreamStickyConversations.mockReturnValue({
      stats: { conversations: [], rangeStart: "", rangeEnd: "" },
      isLoading: false,
      error: null,
    });

    render();

    clickButton(/Edit routing settings/i);
    setInputValue('input[name="secondarySyncIntervalSecs"]', "2400");
    clickButton(/Save settings/i);
    await flushAsync();

    expect(saveRouting).toHaveBeenCalledWith({
      maintenance: {
        primarySyncIntervalSecs: 300,
        secondarySyncIntervalSecs: 2400,
        priorityAvailableAccountCap: 100,
      },
    });
  });

  it("blocks invalid tiered maintenance values before saving", async () => {
    const saveRouting = vi.fn().mockResolvedValue(undefined);

    hookMocks.useUpstreamAccounts.mockReturnValue({
      items: [],
      writesEnabled: true,
      selectedId: null,
      selectedSummary: null,
      detail: null,
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
      runSync: vi.fn(),
      removeAccount: vi.fn(),
      routing: {
        apiKeyConfigured: true,
        maskedApiKey: "pool-live••••",
        maintenance: {
          primarySyncIntervalSecs: 300,
          secondarySyncIntervalSecs: 1800,
          priorityAvailableAccountCap: 100,
        },
      },
      groups: [],
    });
    hookMocks.useUpstreamStickyConversations.mockReturnValue({
      stats: { conversations: [], rangeStart: "", rangeEnd: "" },
      isLoading: false,
      error: null,
    });

    render();

    clickButton(/Edit routing settings/i);
    setInputValue('input[name="primarySyncIntervalSecs"]', "3600");
    setInputValue('input[name="secondarySyncIntervalSecs"]', "300");

    const saveButton = findButton(/Save settings/i);
    expect(saveButton).toBeInstanceOf(HTMLButtonElement);
    expect((saveButton as HTMLButtonElement).disabled).toBe(true);
    expect(saveRouting).not.toHaveBeenCalled();
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
    clickFirstRosterRow();
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

  it("keeps the detail drawer closable while sync is pending", async () => {
    const syncDeferred = deferred<void>();
    const runSync = vi.fn(() => syncDeferred.promise);
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

    hookMocks.useUpstreamAccounts.mockReturnValue({
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
      selectedId: 5,
      selectedSummary: {
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
      detail: {
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
      runSync,
      removeAccount: vi.fn(),
      routing: { apiKeyConfigured: false, maskedApiKey: null },
      groups: [],
      saveGroupNote: vi.fn(),
    });
    hookMocks.useUpstreamStickyConversations.mockReturnValue({
      stats: { conversations: [], rangeStart: "", rangeEnd: "" },
      isLoading: false,
      error: null,
    });

    render();

    clickFirstRosterRow();
    clickButton(/Sync now/i);
    await flushAsync();

    const closeButton = document.body.querySelector(
      '.drawer-header button[type="button"]',
    );

    expect(closeButton).toBeInstanceOf(HTMLButtonElement);
    expect((closeButton as HTMLButtonElement).disabled).toBe(false);

    pressButton(closeButton as HTMLButtonElement);
    await flushAsync();

    expect(document.body.querySelector('[role="dialog"]')).toBeNull();

    syncDeferred.resolve();
    await flushAsync();
  });

  it("keeps refresh enabled while an account action is pending", () => {
    const runSync = vi.fn().mockImplementation(() => new Promise(() => {}));
    const refresh = vi.fn();
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

    hookMocks.useUpstreamAccounts.mockReturnValue({
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
      selectedId: 5,
      selectedSummary: {
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
      detail: {
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
        history: [],
      },
      isLoading: false,
      isDetailLoading: false,
      listError: null,
      detailError: null,
      error: null,
      selectAccount: vi.fn(),
      refresh,
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
    });
    hookMocks.useUpstreamStickyConversations.mockReturnValue({
      stats: { conversations: [], rangeStart: "", rangeEnd: "" },
      isLoading: false,
      error: null,
    });

    render();
    clickFirstRosterRow();
    clickButton(/Sync now/i);

    const refreshButton = findButton(/Refresh/i);
    expect(refreshButton).toBeInstanceOf(HTMLButtonElement);
    expect(refreshButton?.disabled).toBe(false);
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
    clickFirstRosterRow();
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
    clickFirstRosterRow();
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
    clickFirstRosterRow();
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
    clickFirstRosterRow();
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
    clickFirstRosterRow();
    await flushAsync();

    expect(document.body.textContent).toContain("Another OAuth");
    expect(document.body.textContent).not.toContain("stale@example.com");
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
          displayStatus: "error_other",
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
        displayStatus: "error_other",
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
        displayStatus: "error_other",
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

    clickFirstRosterRow();
    expect(document.body.textContent).toContain(
      "This OAuth account still shows a legacy bridge error",
    );
    expect(document.body.textContent).toContain(
      "The stored last_error came from the removed OAuth bridge path",
    );
    expect(document.body.textContent).toContain("Other error");
    expect(document.body.textContent).not.toContain(
      "This OAuth account needs a fresh sign-in",
    );
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
          displayStatus: "needs_reauth",
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
        displayStatus: "needs_reauth",
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
        displayStatus: "needs_reauth",
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

    clickFirstRosterRow();
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
          displayStatus: "upstream_rejected",
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
        displayStatus: "upstream_rejected",
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
        displayStatus: "upstream_rejected",
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

    clickFirstRosterRow();
    expect(document.body.textContent).toContain(
      "The OAuth data plane rejected this request",
    );
    expect(document.body.textContent).toContain("Upstream rejected");
    expect(document.body.textContent).not.toContain(
      "This OAuth account needs a fresh sign-in",
    );
  });
});

describe("UpstreamAccountsPage api key details", () => {
  it("does not clear another account's pending api key draft when an earlier save resolves", async () => {
    const saveRequest = deferred();
    const saveAccount = vi.fn().mockImplementation(() => saveRequest.promise);
    const baseItems = [
      {
        id: 8,
        kind: "api_key_codex" as const,
        provider: "codex" as const,
        displayName: "Gateway Key",
        groupName: "prod",
        isMother: false,
        status: "active",
        enabled: true,
        maskedApiKey: "sk-gate••••",
      },
      {
        id: 9,
        kind: "api_key_codex" as const,
        provider: "codex" as const,
        displayName: "Backup Key",
        groupName: "prod",
        isMother: false,
        status: "active",
        enabled: true,
        maskedApiKey: "sk-back••••",
      },
    ];
    const detailFor = (id: number, displayName: string, upstreamBaseUrl: string | null) => ({
      id,
      kind: "api_key_codex" as const,
      provider: "codex" as const,
      displayName,
      groupName: "prod",
      isMother: false,
      status: "active",
      enabled: true,
      history: [],
      note: null,
      upstreamBaseUrl,
      localLimits: {
        primaryLimit: 100,
        secondaryLimit: 1000,
        limitUnit: "requests",
      },
    });
    let state = {
      items: baseItems,
      writesEnabled: true,
      selectedId: 8,
      selectedSummary: baseItems[0],
      detail: detailFor(8, "Gateway Key", "https://proxy.example.com/gateway"),
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
      saveAccount,
      saveRouting: vi.fn(),
      runSync: vi.fn(),
      removeAccount: vi.fn(),
      routing: { apiKeyConfigured: true, maskedApiKey: "pool-live••••" },
      groups: [],
      saveGroupNote: vi.fn(),
    };
    hookMocks.useUpstreamAccounts.mockImplementation(() => state);
    hookMocks.useUpstreamStickyConversations.mockReturnValue({
      stats: { conversations: [], rangeStart: "", rangeEnd: "" },
      isLoading: false,
      error: null,
    });

    render();

    clickFirstRosterRow();
    clickButton(/Save changes/i);
    await flushAsync();
    expect(saveAccount).toHaveBeenCalledWith(8, expect.any(Object));

    state = {
      ...state,
      selectedId: 9,
      selectedSummary: baseItems[1],
      detail: detailFor(9, "Backup Key", "https://proxy.example.com/backup"),
    };
    rerender();
    await flushAsync();

    setInputValue('input[name="detailRotateApiKey"]', "sk-new-backup");
    saveRequest.resolve(detailFor(8, "Gateway Key", "https://proxy.example.com/gateway"));
    await flushAsync();

    const rotateInput = document.body.querySelector('input[name="detailRotateApiKey"]');
    expect(rotateInput).toBeInstanceOf(HTMLInputElement);
    expect((rotateInput as HTMLInputElement).value).toBe("sk-new-backup");
  });

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

    clickFirstRosterRow();
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

    clickFirstRosterRow();
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

    clickFirstRosterRow();
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
  it("does not close the current drawer when an earlier account delete resolves after switching accounts", async () => {
    const removeRequest = deferred<void>();
    const removeAccount = vi.fn().mockImplementation(() => removeRequest.promise);
    const baseItems = [
      {
        id: 8,
        kind: "api_key_codex" as const,
        provider: "codex" as const,
        displayName: "Gateway Key",
        groupName: "prod",
        isMother: false,
        status: "active",
        enabled: true,
        maskedApiKey: "sk-gate••••",
      },
      {
        id: 9,
        kind: "api_key_codex" as const,
        provider: "codex" as const,
        displayName: "Backup Key",
        groupName: "prod",
        isMother: false,
        status: "active",
        enabled: true,
        maskedApiKey: "sk-back••••",
      },
    ];
    const detailFor = (id: number, displayName: string) => ({
      id,
      kind: "api_key_codex" as const,
      provider: "codex" as const,
      displayName,
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
    let state = {
      items: baseItems,
      writesEnabled: true,
      selectedId: 8,
      selectedSummary: baseItems[0],
      detail: detailFor(8, "Gateway Key"),
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
      removeAccount,
      routing: { apiKeyConfigured: true, maskedApiKey: "pool-live••••" },
      groups: [],
      saveGroupNote: vi.fn(),
    };
    hookMocks.useUpstreamAccounts.mockImplementation(() => state);
    hookMocks.useUpstreamStickyConversations.mockReturnValue({
      stats: { conversations: [], rangeStart: "", rangeEnd: "" },
      isLoading: false,
      error: null,
    });

    render();

    clickFirstRosterRow();
    clickButton(/^Delete$/i);
    await flushAsync();
    clickButton(/Delete account/i);
    await flushAsync();
    expect(removeAccount).toHaveBeenCalledWith(8);

    state = {
      ...state,
      selectedId: 9,
      selectedSummary: baseItems[1],
      detail: detailFor(9, "Backup Key"),
    };
    rerender();
    await flushAsync();

    removeRequest.resolve();
    await flushAsync();

    const dialogTitle = document.body.querySelector("#upstream-account-detail-title");
    expect(dialogTitle?.textContent).toBe("Backup Key");
    expect(document.body.querySelector('[role="dialog"]')).not.toBeNull();
  });

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

    clickFirstRosterRow();
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

    clickFirstRosterRow();
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

    clickFirstRosterRow();
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

    clickFirstRosterRow();
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

  it("keeps the tag picker popover inside the detail drawer dialog subtree", async () => {
    mockAccountsPage();
    render({
      pathname: "/account-pool/upstream-accounts",
      state: {
        selectedAccountId: 5,
        openDetail: true,
      },
    });
    await flushAsync();
    await flushTimers();

    clickButton(/^add tag$/i);
    await flushAsync();
    await flushTimers();

    const searchInput = document.body.querySelector(
      'input[placeholder="Search existing tags..."]',
    );
    expect(searchInput).not.toBeNull();
    expect(searchInput?.closest('[role="dialog"]')).not.toBeNull();
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

    clickFirstRosterRow();
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
