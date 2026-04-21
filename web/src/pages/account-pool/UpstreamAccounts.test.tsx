/* eslint-disable @typescript-eslint/ban-ts-comment, @typescript-eslint/no-unused-vars */
// @ts-nocheck
import ts from "typescript";
import suite1 from "./UpstreamAccounts.roster-freshness.txt?raw";
import suite2 from "./UpstreamAccounts.duplicates.txt?raw";
import suite3 from "./UpstreamAccounts.sync-state-isolation.txt?raw";
import suite4 from "./UpstreamAccounts.oauth-recovery.txt?raw";
import suite5 from "./UpstreamAccounts.api-key-details.txt?raw";
import suite6 from "./UpstreamAccounts.delete-confirmation.txt?raw";
import suite7 from "./UpstreamAccounts.edit-drafts.txt?raw";

/** @vitest-environment jsdom */
import * as React from "react";
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
import {
  createMemoryRouter,
  MemoryRouter,
  RouterProvider,
  Route,
  Routes,
  type InitialEntry,
} from "react-router-dom";
import { SystemNotificationProvider } from "../../components/ui/system-notifications";
import { I18nProvider } from "../../i18n";
import UpstreamAccountsPage, {
  SharedUpstreamAccountDetailDrawer,
} from "./UpstreamAccounts";
import type { EffectiveRoutingRule, TagSummary } from "../../lib/api";

type UpstreamAccountsHookValue = ReturnType<
  typeof import("../../hooks/useUpstreamAccounts").useUpstreamAccounts
>;
type DeleteRegressionState = Partial<UpstreamAccountsHookValue> & {
  selectedId: number | null;
  selectedSummary: UpstreamAccountsHookValue["selectedSummary"] | null;
  detail: UpstreamAccountsHookValue["detail"] | null;
  routing: UpstreamAccountsHookValue["routing"];
};

const UPSTREAM_ACCOUNTS_FILTER_STORAGE_KEY =
  "codex-vibe-monitor.account-pool.upstream-accounts.filters";
const LOCALE_STORAGE_KEY = "codex-vibe-monitor.locale";
const navigateMock = vi.hoisted(() => vi.fn());
const hookMocks = vi.hoisted(() => ({
  useUpstreamAccounts: vi.fn(),
  useForwardProxyBindingNodes: vi.fn(),
  useUpstreamStickyConversations: vi.fn(),
  usePoolTags: vi.fn(),
}));
const apiMocks = vi.hoisted(() => ({
  createBulkUpstreamAccountSyncJobEventSource: vi.fn(),
}));
const storage = new Map<string, string>();

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

vi.mock("../../hooks/useUpstreamStickyConversations", () => ({
  useUpstreamStickyConversations: hookMocks.useUpstreamStickyConversations,
}));

vi.mock("../../hooks/usePoolTags", () => ({
  usePoolTags: hookMocks.usePoolTags,
}));

vi.mock("../../lib/api", async () => {
  const actual =
    await vi.importActual<typeof import("../../lib/api")>("../../lib/api");
  return {
    ...actual,
    createBulkUpstreamAccountSyncJobEventSource:
      apiMocks.createBulkUpstreamAccountSyncJobEventSource,
  };
});

let host: HTMLDivElement | null = null;
let root: Root | null = null;

class MockBulkSyncEventSource implements EventTarget {
  private listeners = new Map<string, Set<EventListener>>();
  readyState = 1;
  onerror: ((this: EventSource, ev: Event) => unknown) | null = null;

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

  fail() {
    if (this.readyState === 2) return;
    this.onerror?.call(this as unknown as EventSource, new Event("error"));
  }
}

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
      getItem: vi.fn((key: string) => storage.get(key) ?? null),
      setItem: vi.fn((key: string, value: string) => {
        storage.set(key, value);
      }),
      removeItem: vi.fn((key: string) => {
        storage.delete(key);
      }),
    },
  });
});

beforeEach(() => {
  storage.clear();
  storage.set(LOCALE_STORAGE_KEY, "en");
  vi.mocked(window.localStorage.getItem).mockImplementation(
    (key: string) => storage.get(key) ?? null,
  );
  vi.mocked(window.localStorage.setItem).mockImplementation(
    (key: string, value: string) => {
      storage.set(key, value);
    },
  );
  vi.mocked(window.localStorage.removeItem).mockImplementation(
    (key: string) => {
      storage.delete(key);
    },
  );
  apiMocks.createBulkUpstreamAccountSyncJobEventSource.mockReset();
  apiMocks.createBulkUpstreamAccountSyncJobEventSource.mockImplementation(
    () => {
      throw new Error("unexpected bulk sync event source");
    },
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
  hookMocks.useForwardProxyBindingNodes.mockReturnValue({
    nodes: [
      {
        key: "__direct__",
        displayName: "Direct",
        protocolLabel: "DIRECT",
        source: "direct",
        penalized: false,
        selectable: true,
        last24h: [],
      },
      {
        key: "jp-edge-01",
        displayName: "JP Edge 01",
        protocolLabel: "HTTP",
        source: "inventory",
        penalized: false,
        selectable: true,
        last24h: [],
      },
      {
        key: "vless://11111111-2222-3333-4444-555555555555@fixture-vless-edge.example.invalid:443?encryption=none&security=tls&type=ws&host=cdn.example.invalid&path=%2Ffixture&fp=chrome&pbk=fixture-public-key&sid=fixture-subscription-node#Ivan-hinet-vless-vision-01KF874741GBN6MQYD6TNMYDVS",
        displayName: "Ivan-hinet-vless-vision-01KF874741GBN6MQYD6TNMYDVS",
        protocolLabel: "VLESS",
        source: "subscription",
        penalized: false,
        selectable: true,
        last24h: [],
      },
    ],
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

function render(
  initialEntry: InitialEntry = "/account-pool/upstream-accounts",
) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  rerender(initialEntry);
}

function rerender(
  initialEntry: InitialEntry = "/account-pool/upstream-accounts",
) {
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

function remount(
  initialEntry: InitialEntry = "/account-pool/upstream-accounts",
) {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  render(initialEntry);
}

function writeStoredUpstreamFilters(payload: unknown) {
  storage.set(
    UPSTREAM_ACCOUNTS_FILTER_STORAGE_KEY,
    typeof payload === "string" ? payload : JSON.stringify(payload),
  );
}

function readStoredUpstreamFilters() {
  const raw = storage.get(UPSTREAM_ACCOUNTS_FILTER_STORAGE_KEY);
  if (!raw) {
    return null;
  }
  return JSON.parse(raw);
}

function expectRosterHookQuery(expected: Record<string, unknown> | null) {
  expect(hookMocks.useUpstreamAccounts.mock.calls).toContainEqual([expected]);
}

function findButton(pattern: RegExp) {
  return Array.from(document.body.querySelectorAll("button")).find(
    (candidate) =>
      pattern.test(
        candidate.textContent || candidate.getAttribute("aria-label") || "",
      ),
  ) as HTMLButtonElement | undefined;
}

function findExactTextElements(text: string, root: ParentNode = document.body) {
  return Array.from(root.querySelectorAll("*")).filter(
    (candidate) =>
      candidate instanceof HTMLElement &&
      candidate.children.length === 0 &&
      candidate.textContent?.trim() === text,
  ) as HTMLElement[];
}

function findFixedContainerByText(pattern: RegExp) {
  return Array.from(document.body.querySelectorAll(".fixed")).find(
    (candidate) => pattern.test(candidate.textContent || ""),
  ) as HTMLElement | undefined;
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

function setFieldValue(
  input: HTMLInputElement | HTMLTextAreaElement,
  value: string,
) {
  const prototype =
    input instanceof HTMLTextAreaElement
      ? HTMLTextAreaElement.prototype
      : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
  if (!setter) throw new Error("missing native setter");
  act(() => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

function setComboboxValue(nameSelector: string, value: string) {
  const hiddenInput = document.body.querySelector(nameSelector);
  if (!(hiddenInput instanceof HTMLInputElement)) {
    throw new Error(`missing combobox input: ${nameSelector}`);
  }
  const wrapper = hiddenInput.parentElement;
  const trigger = wrapper?.querySelector('button[role="combobox"]');
  if (!(trigger instanceof HTMLButtonElement)) {
    throw new Error(`missing combobox trigger: ${nameSelector}`);
  }
  pressButton(trigger);

  const searchInput = document.body.querySelector("[cmdk-input]");
  if (!(searchInput instanceof HTMLInputElement)) {
    throw new Error(`missing command input: ${nameSelector}`);
  }
  setFieldValue(searchInput, value);

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

function clickTab(matcher: RegExp) {
  const tab = Array.from(document.body.querySelectorAll('[role="tab"]')).find(
    (candidate) =>
      candidate instanceof HTMLButtonElement &&
      matcher.test(
        candidate.textContent || candidate.getAttribute("aria-label") || "",
      ),
  );
  if (!(tab instanceof HTMLButtonElement)) {
    throw new Error(`missing tab: ${matcher}`);
  }
  pressButton(tab);
  return tab;
}

function clickDrawerBackdrop() {
  const overlay =
    document.body.querySelector(".drawer-shell")?.parentElement
      ?.previousElementSibling;
  if (!(overlay instanceof HTMLElement)) {
    throw new Error("missing drawer backdrop");
  }
  act(() => {
    overlay.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  return overlay;
}

function clickDrawerGutter() {
  const gutter = document.body.querySelector(".drawer-shell")?.parentElement;
  if (!(gutter instanceof HTMLElement)) {
    throw new Error("missing drawer gutter");
  }
  act(() => {
    gutter.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  return gutter;
}

function clickFirstRosterRow() {
  const row = document.body.querySelector('tbody tr[role="button"]');
  if (!(row instanceof HTMLTableRowElement)) {
    throw new Error("missing roster row");
  }
  act(() => {
    row.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  return row;
}

function clickCheckboxByLabel(matcher: RegExp) {
  const checkbox = Array.from(
    document.body.querySelectorAll('input[type="checkbox"]'),
  ).find(
    (candidate) =>
      candidate instanceof HTMLInputElement &&
      matcher.test(candidate.getAttribute("aria-label") || ""),
  );
  if (!(checkbox instanceof HTMLInputElement)) {
    throw new Error(`missing checkbox: ${matcher}`);
  }
  act(() => {
    checkbox.click();
  });
  return checkbox;
}

function clickCombobox(matcher: RegExp) {
  const trigger = Array.from(
    document.body.querySelectorAll('button[role="combobox"]'),
  ).find(
    (candidate) =>
      candidate instanceof HTMLButtonElement &&
      matcher.test(
        candidate.getAttribute("aria-label") || candidate.textContent || "",
      ),
  );
  if (!(trigger instanceof HTMLButtonElement)) {
    throw new Error(`missing combobox: ${matcher}`);
  }
  pressButton(trigger);
  return trigger;
}

function clickCommandItem(matcher: RegExp) {
  const item = Array.from(document.body.querySelectorAll("[cmdk-item]")).find(
    (candidate) =>
      candidate instanceof HTMLElement &&
      matcher.test(candidate.textContent || ""),
  );
  if (!(item instanceof HTMLElement)) {
    throw new Error(`missing command item: ${matcher}`);
  }
  act(() => {
    item.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  return item;
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
};

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
];

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function buildBulkSyncCounts(rows: Array<{ status: string }>) {
  return rows.reduce(
    (counts, row) => {
      counts.total += 1;
      if (row.status === "succeeded") {
        counts.completed += 1;
        counts.succeeded += 1;
      } else if (row.status === "failed") {
        counts.completed += 1;
        counts.failed += 1;
      } else if (row.status === "skipped") {
        counts.completed += 1;
        counts.skipped += 1;
      }
      return counts;
    },
    {
      total: 0,
      completed: 0,
      succeeded: 0,
      failed: 0,
      skipped: 0,
    },
  );
}

function buildBulkSyncSnapshot(
  jobId: string,
  rows: Array<{
    accountId: number;
    displayName: string;
    status: string;
    detail?: string | null;
  }>,
  status = "running",
) {
  return {
    jobId,
    status,
    rows,
  };
}

function buildBulkSyncSnapshotEvent(
  jobId: string,
  rows: Array<{
    accountId: number;
    displayName: string;
    status: string;
    detail?: string | null;
  }>,
  status = "running",
) {
  return {
    snapshot: buildBulkSyncSnapshot(jobId, rows, status),
    counts: buildBulkSyncCounts(rows),
  };
}

function buildBulkSyncJobResponse(
  jobId: string,
  rows: Array<{
    accountId: number;
    displayName: string;
    status: string;
    detail?: string | null;
  }>,
  status = "running",
) {
  return {
    jobId,
    ...buildBulkSyncSnapshotEvent(jobId, rows, status),
  };
}

function mockBulkSyncPage(options?: {
  refresh?: ReturnType<typeof vi.fn>;
  startBulkSyncJob?: ReturnType<typeof vi.fn>;
  getBulkSyncJob?: ReturnType<typeof vi.fn>;
  stopBulkSyncJob?: ReturnType<typeof vi.fn>;
}) {
  const refresh = options?.refresh ?? vi.fn();
  const startBulkSyncJob =
    options?.startBulkSyncJob ??
    vi.fn().mockResolvedValue(
      buildBulkSyncJobResponse("job-1", [
        { accountId: 5, displayName: "Existing OAuth", status: "pending" },
        { accountId: 9, displayName: "Another OAuth", status: "pending" },
      ]),
    );
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
    selectedSummary: null,
    detail: null,
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
    getBulkSyncJob: options?.getBulkSyncJob ?? vi.fn(),
    stopBulkSyncJob: options?.stopBulkSyncJob ?? vi.fn(),
    runSync: vi.fn(),
    removeAccount: vi.fn(),
    groups: [],
    routing: { apiKeyConfigured: false, maskedApiKey: null },
  });
  return { refresh, startBulkSyncJob };
}

function mockAccountsPage(options?: {
  saveRouting?: ReturnType<typeof vi.fn>;
  routing?: {
    writesEnabled: boolean;
    apiKeyConfigured: boolean;
    maskedApiKey: string | null;
    timeouts: {
      responsesFirstByteTimeoutSecs: number;
      compactFirstByteTimeoutSecs: number;
      responsesStreamTimeoutSecs: number;
      compactStreamTimeoutSecs: number;
    };
  } | null;
  item?: Record<string, unknown>;
  selectedSummary?: Record<string, unknown>;
  detail?: Record<string, unknown>;
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
      options && "routing" in options
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

function mockRosterFreshnessPage(options?: {
  listState?: {
    queryKey: string | null;
    dataQueryKey: string | null;
    freshness: "fresh" | "stale" | "missing" | "deferred";
    loadingState: "idle" | "deferred" | "initial" | "switching" | "refreshing";
    status: "ready" | "loading" | "error" | "deferred";
    hasCurrentQueryData: boolean;
    isPending: boolean;
  };
  listError?: string | null;
  refresh?: ReturnType<typeof vi.fn>;
}) {
  const refresh = options?.refresh ?? vi.fn();
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
        isMother: false,
        tags: [],
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
    selectedId: null,
    selectedSummary: null,
    detail: null,
    isLoading: false,
    isDetailLoading: false,
    listError: options?.listError ?? null,
    listState: options?.listState ?? {
      queryKey: "roster-q1",
      dataQueryKey: "roster-q1",
      freshness: "fresh",
      loadingState: "idle",
      status: "ready",
      hasCurrentQueryData: true,
      isPending: false,
    },
    detailError: null,
    error: options?.listError ?? null,
    selectAccount: vi.fn(),
    refresh,
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
    routing: {
      writesEnabled: true,
      apiKeyConfigured: false,
      maskedApiKey: null,
    },
  });
  return { refresh };
}

const scope: Record<string, unknown> = {};
Object.assign(scope, {
  act,
  React,
  createRoot,
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
  MemoryRouter,
  createMemoryRouter,
  RouterProvider,
  Route,
  Routes,
  SystemNotificationProvider,
  I18nProvider,
  UpstreamAccountsPage,
  SharedUpstreamAccountDetailDrawer,
  UPSTREAM_ACCOUNTS_FILTER_STORAGE_KEY,
  LOCALE_STORAGE_KEY,
  navigateMock,
  hookMocks,
  apiMocks,
  storage,
  MockBulkSyncEventSource,
  render,
  rerender,
  remount,
  flushAsync,
  flushTimers,
  writeStoredUpstreamFilters,
  readStoredUpstreamFilters,
  expectRosterHookQuery,
  findButton,
  findExactTextElements,
  findFixedContainerByText,
  setInputValue,
  setFieldValue,
  setComboboxValue,
  clickButton,
  clickTab,
  clickDrawerBackdrop,
  clickDrawerGutter,
  clickFirstRosterRow,
  clickCheckboxByLabel,
  clickCombobox,
  clickCommandItem,
  pressButton,
  defaultEffectiveRoutingRule,
  defaultPoolTags,
  deferred,
  buildBulkSyncCounts,
  buildBulkSyncSnapshot,
  buildBulkSyncSnapshotEvent,
  buildBulkSyncJobResponse,
  mockBulkSyncPage,
  mockAccountsPage,
  mockRosterFreshnessPage,
});
Object.defineProperties(scope, {
  host: { get: () => host, set: (value) => { host = value as typeof host; } },
  root: { get: () => root, set: (value) => { root = value as typeof root; } },
});
const evalChunk = (chunk: string) => {
  const { outputText } = ts.transpileModule(chunk, {
    compilerOptions: {
      jsx: ts.JsxEmit.React,
      module: ts.ModuleKind.None,
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

describe('UpstreamAccountsPage grouped roster toggle', () => {
  it('switches to grouped view and hides the pagination footer', async () => {
    mockRosterFreshnessPage()
    render()
    await act(async () => {
      await Promise.resolve()
    })

    const groupedToggle = Array.from(
      host?.querySelectorAll('button[role="tab"]') ?? [],
    ).find((candidate) => /grouped|分组/i.test(candidate.textContent ?? ''))
    expect(groupedToggle).toBeTruthy()

    act(() => {
      groupedToggle?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    expect(
      host?.querySelector('[data-testid="upstream-accounts-grouped-roster"]'),
    ).toBeTruthy()
    expect(
      Array.from(host?.querySelectorAll('input[type="checkbox"]') ?? []).find(
        (candidate) =>
          /select current page|选择当前页/i.test(
            candidate.getAttribute('aria-label') ?? '',
          ),
      ),
    ).toBeTruthy()
    expect(
      host?.querySelector('[data-testid="upstream-accounts-pagination-footer"]'),
    ).toBeNull()
  })

  it('switches to grid view without bulk selection controls', async () => {
    mockRosterFreshnessPage()
    render()
    await act(async () => {
      await Promise.resolve()
    })

    const gridToggle = Array.from(
      host?.querySelectorAll('button[role="tab"]') ?? [],
    ).find((candidate) => /grid|网格/i.test(candidate.textContent ?? ''))
    expect(gridToggle).toBeTruthy()

    act(() => {
      gridToggle?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    expect(
      host?.querySelector('[data-testid="upstream-accounts-grouped-roster"]'),
    ).toBeTruthy()
    expect(gridToggle?.getAttribute('aria-selected')).toBe('true')
    expect(
      Array.from(host?.querySelectorAll('input[type="checkbox"]') ?? []).find(
        (candidate) =>
          /select current page|选择当前页/i.test(
            candidate.getAttribute('aria-label') ?? '',
          ),
      ),
    ).toBeFalsy()
    expect(
      host?.querySelector('[data-testid="upstream-accounts-pagination-footer"]'),
    ).toBeNull()
  })

  it('blocks grouped roster interactions while the include-all query is still switching', async () => {
    mockRosterFreshnessPage({
      listState: {
        queryKey: '{"includeAll":true}',
        dataQueryKey: '{"page":2,"pageSize":20}',
        freshness: 'stale',
        loadingState: 'switching',
        status: 'ready',
        hasCurrentQueryData: true,
        isPending: true,
      },
    })
    render()
    await act(async () => {
      await Promise.resolve()
    })

    const groupedToggle = Array.from(
      host?.querySelectorAll('button[role="tab"]') ?? [],
    ).find((candidate) => /grouped|分组/i.test(candidate.textContent ?? ''))
    expect(groupedToggle).toBeTruthy()

    act(() => {
      groupedToggle?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    expect(
      host?.querySelector('[data-testid="upstream-accounts-grouped-loading"]'),
    ).toBeTruthy()
    expect(
      host?.querySelector('[data-testid="upstream-accounts-grouped-roster"]'),
    ).toBeNull()
    expect(
      Array.from(host?.querySelectorAll('input[type="checkbox"]') ?? []).find(
        (candidate) =>
          /select current page|选择当前页/i.test(
            candidate.getAttribute('aria-label') ?? '',
          ),
      ),
    ).toBeFalsy()
  })

  it('blocks flat roster interactions while switching away from grouped include-all data', async () => {
    mockRosterFreshnessPage({
      listState: {
        queryKey: '{"page":2,"pageSize":20}',
        dataQueryKey: '{"includeAll":true}',
        freshness: 'stale',
        loadingState: 'switching',
        status: 'ready',
        hasCurrentQueryData: true,
        isPending: true,
      },
    })
    render()
    await act(async () => {
      await Promise.resolve()
    })

    expect(
      host?.querySelector('[data-testid="upstream-accounts-table-loading"]'),
    ).toBeTruthy()
    expect(
      host?.querySelector('[data-testid="upstream-accounts-pagination-footer"]'),
    ).toBeTruthy()
    expect(host?.textContent ?? '').not.toContain('Existing OAuth')
    expect(host?.textContent ?? '').not.toContain('Another OAuth')
  })

  it('preserves the flat page selection when toggling grouped view', async () => {
    const items = [
      {
        id: 5,
        kind: 'oauth_codex',
        provider: 'codex',
        displayName: 'Existing OAuth',
        groupName: 'prod',
        status: 'active',
        displayStatus: 'active',
        enabled: true,
        isMother: false,
        tags: [],
        effectiveRoutingRule: defaultEffectiveRoutingRule,
      },
    ]
    const sharedCallbacks = {
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
    }
    const sharedRouting = {
      writesEnabled: true,
      apiKeyConfigured: false,
      maskedApiKey: null,
    }
    const sharedBase = {
      items,
      hasUngroupedAccounts: true,
      writesEnabled: true,
      total: 45,
      metrics: {
        total: 45,
        oauth: 45,
        apiKey: 0,
        attention: 0,
      },
      selectedId: null,
      selectedSummary: null,
      detail: null,
      isLoading: false,
      isDetailLoading: false,
      listError: null,
      detailError: null,
      error: null,
      groups: [],
      routing: sharedRouting,
      ...sharedCallbacks,
    }
    const flatPage1 = {
      ...sharedBase,
      page: 1,
      pageSize: 20,
      listState: {
        queryKey: '{"page":1,"pageSize":20}',
        dataQueryKey: '{"page":1,"pageSize":20}',
        freshness: 'fresh',
        loadingState: 'idle',
        status: 'ready',
        hasCurrentQueryData: true,
        isPending: false,
      },
    }
    const flatPage2 = {
      ...sharedBase,
      page: 2,
      pageSize: 20,
      listState: {
        queryKey: '{"page":2,"pageSize":20}',
        dataQueryKey: '{"page":2,"pageSize":20}',
        freshness: 'fresh',
        loadingState: 'idle',
        status: 'ready',
        hasCurrentQueryData: true,
        isPending: false,
      },
    }
    const groupedAll = {
      ...sharedBase,
      page: 1,
      pageSize: 45,
      listState: {
        queryKey: '{"includeAll":true}',
        dataQueryKey: '{"includeAll":true}',
        freshness: 'fresh',
        loadingState: 'idle',
        status: 'ready',
        hasCurrentQueryData: true,
        isPending: false,
      },
    }
    hookMocks.useUpstreamAccounts.mockImplementation((query) => {
      if (query?.includeAll) return groupedAll
      if (query?.page === 2) return flatPage2
      return flatPage1
    })

    render()
    await act(async () => {
      await Promise.resolve()
    })

    const nextButton = findButton(/next|下一页/i)
    expect(nextButton).toBeTruthy()
    act(() => {
      nextButton?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    expectRosterHookQuery({ page: 2, pageSize: 20 })

    const groupedToggle = Array.from(
      host?.querySelectorAll('button[role="tab"]') ?? [],
    ).find((candidate) => /grouped|分组/i.test(candidate.textContent ?? ''))
    const flatToggle = Array.from(
      host?.querySelectorAll('button[role="tab"]') ?? [],
    ).find((candidate) => /flat|平铺/i.test(candidate.textContent ?? ''))
    expect(groupedToggle).toBeTruthy()
    expect(flatToggle).toBeTruthy()

    act(() => {
      groupedToggle?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    expectRosterHookQuery({ includeAll: true })

    act(() => {
      flatToggle?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    expectRosterHookQuery({ page: 2, pageSize: 20 })
  })
})
