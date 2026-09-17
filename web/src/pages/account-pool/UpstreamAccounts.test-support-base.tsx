import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { type InitialEntry, MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, beforeAll, beforeEach, expect, vi } from "vitest";
import { SystemNotificationProvider } from "../../components/ui/system-notifications";
import { COMPACT_VIEWPORT_MEDIA_QUERY } from "../../hooks/useCompactViewport";
import { I18nProvider } from "../../i18n";
import type { BroadcastPayload, EffectiveRoutingRule, TagSummary } from "../../lib/api";
import UpstreamAccountsPage from "./UpstreamAccounts";

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
  fetchUpstreamAccountActionEvents: vi.fn(),
  fetchInvocationRecordLocation: vi.fn(),
  fetchInvocationRecords: vi.fn(),
}));
const sseMocks = vi.hoisted(() => ({
  onMessage: null as null | ((payload: BroadcastPayload) => void),
  onOpen: null as null | (() => void),
}));
const virtualizerMocks = vi.hoisted(() => ({
  visibleIndexes: null as number[] | null,
  scrollToIndex: vi.fn(),
}));
const storage = new Map<string, string>();
let compactViewportMatches = false;
vi.mock("react-router-dom", async () => {
  const actual = await vi.importActual<typeof import("react-router-dom")>("react-router-dom");
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
  const actual = await vi.importActual<typeof import("../../lib/api")>("../../lib/api");
  return {
    ...actual,
    createBulkUpstreamAccountSyncJobEventSource:
      apiMocks.createBulkUpstreamAccountSyncJobEventSource,
    fetchUpstreamAccountActionEvents: apiMocks.fetchUpstreamAccountActionEvents,
    fetchInvocationRecordLocation: apiMocks.fetchInvocationRecordLocation,
    fetchInvocationRecords: apiMocks.fetchInvocationRecords,
  };
});
vi.mock("../../lib/sse", () => ({
  subscribeToSse: (handler: (payload: BroadcastPayload) => void) => {
    sseMocks.onMessage = handler;
    return () => {
      sseMocks.onMessage = null;
    };
  },
  subscribeToSseOpen: (handler: () => void) => {
    sseMocks.onOpen = handler;
    return () => {
      sseMocks.onOpen = null;
    };
  },
}));
vi.mock("../../features/dashboard/DashboardActivityOverview", () => ({
  DashboardActivityOverview: ({
    testId,
    upstreamAccountId,
  }: {
    testId?: string;
    upstreamAccountId?: number | null;
  }) => (
    <section
      data-testid={testId ?? "dashboard-activity-overview"}
      data-upstream-account-id={upstreamAccountId ?? ""}
    >
      Account activity overview
    </section>
  ),
}));
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({
    count,
    estimateSize,
    scrollMargin = 0,
  }: {
    count: number;
    estimateSize: (index: number) => number;
    scrollMargin?: number;
  }) => {
    const sizes = Array.from({ length: count }, (_, index) => estimateSize(index));
    const indexes =
      virtualizerMocks.visibleIndexes ??
      Array.from({ length: Math.min(count, 4) }, (_, index) => index);
    const items = indexes
      .filter((index) => index >= 0 && index < count)
      .map((index) => {
        const size = sizes[index] ?? estimateSize(index);
        return {
          key: index,
          index,
          start:
            scrollMargin +
            sizes.slice(0, index).reduce((sum, candidateSize) => sum + candidateSize, 0),
          size,
          end:
            scrollMargin +
            sizes.slice(0, index + 1).reduce((sum, candidateSize) => sum + candidateSize, 0),
        };
      });
    return {
      measureElement: () => undefined,
      measure: () => undefined,
      getVirtualItems: () => items,
      getTotalSize: () => sizes.reduce((sum, size) => sum + size, 0),
      scrollToIndex: virtualizerMocks.scrollToIndex,
    };
  },
  useWindowVirtualizer: ({
    count,
    estimateSize,
    scrollMargin = 0,
  }: {
    count: number;
    estimateSize: (index: number) => number;
    scrollMargin?: number;
  }) => {
    const sizes = Array.from({ length: count }, (_, index) => estimateSize(index));
    const indexes =
      virtualizerMocks.visibleIndexes ??
      Array.from({ length: Math.min(count, 4) }, (_, index) => index);
    const items = indexes
      .filter((index) => index >= 0 && index < count)
      .map((index) => {
        const size = sizes[index] ?? estimateSize(index);
        return {
          key: index,
          index,
          start:
            scrollMargin +
            sizes.slice(0, index).reduce((sum, candidateSize) => sum + candidateSize, 0),
          size,
          end:
            scrollMargin +
            sizes.slice(0, index + 1).reduce((sum, candidateSize) => sum + candidateSize, 0),
        };
      });
    return {
      measureElement: () => undefined,
      measure: () => undefined,
      getVirtualItems: () => items,
      getTotalSize: () => sizes.reduce((sum, size) => sum + size, 0),
      scrollToIndex: virtualizerMocks.scrollToIndex,
    };
  },
}));
let host: HTMLDivElement | null = null;
let root: Root | null = null;

function setHost(value: HTMLDivElement | null) {
  host = value;
}

function setRoot(value: Root | null) {
  root = value;
}
class MockBulkSyncEventSource implements EventTarget {
  private listeners = new Map<string, Set<EventListener>>();
  readyState = 1;
  onerror: ((this: EventSource, ev: Event) => unknown) | null = null;
  addEventListener(type: string, listener: EventListenerOrEventListenerObject | null) {
    if (!listener) return;
    const handler =
      typeof listener === "function"
        ? listener
        : (((event: Event) => listener.handleEvent(event)) as EventListener);
    const current = this.listeners.get(type) ?? new Set<EventListener>();
    current.add(handler);
    this.listeners.set(type, current);
  }
  removeEventListener(type: string, listener: EventListenerOrEventListenerObject | null) {
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
    current.forEach((listener) => {
      listener(event);
    });
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
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: vi.fn().mockImplementation((query: string) => ({
      matches: query === COMPACT_VIEWPORT_MEDIA_QUERY ? compactViewportMatches : false,
      media: query,
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
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
  compactViewportMatches = false;
  virtualizerMocks.visibleIndexes = null;
  virtualizerMocks.scrollToIndex.mockReset();
  storage.clear();
  storage.set(LOCALE_STORAGE_KEY, "en");
  vi.mocked(window.localStorage.getItem).mockImplementation(
    (key: string) => storage.get(key) ?? null,
  );
  vi.mocked(window.localStorage.setItem).mockImplementation((key: string, value: string) => {
    storage.set(key, value);
  });
  vi.mocked(window.localStorage.removeItem).mockImplementation((key: string) => {
    storage.delete(key);
  });
  apiMocks.createBulkUpstreamAccountSyncJobEventSource.mockReset();
  apiMocks.createBulkUpstreamAccountSyncJobEventSource.mockImplementation(() => {
    throw new Error("unexpected bulk sync event source");
  });
  apiMocks.fetchUpstreamAccountActionEvents.mockReset();
  apiMocks.fetchUpstreamAccountActionEvents.mockResolvedValue({
    items: [],
    total: 0,
    page: 1,
    pageSize: 20,
  });
  apiMocks.fetchInvocationRecords.mockReset();
  apiMocks.fetchInvocationRecords.mockResolvedValue({
    snapshotId: 42,
    total: 1,
    page: 1,
    pageSize: 50,
    records: [],
  });
  apiMocks.fetchInvocationRecordLocation.mockReset();
  sseMocks.onMessage = null;
  sseMocks.onOpen = null;
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
  sseMocks.onMessage = null;
  sseMocks.onOpen = null;
});
function render(initialEntry: InitialEntry = "/account-pool/upstream-accounts") {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  rerender(initialEntry);
}
function renderFlatForLegacySuites(initialEntry: InitialEntry = "/account-pool/upstream-accounts") {
  render(initialEntry);
  const flatToggle = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
    (candidate) => /flat|平铺/i.test(candidate.textContent ?? ""),
  );
  if (flatToggle instanceof HTMLButtonElement) {
    act(() => {
      flatToggle.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
  }
}
function rerender(initialEntry: InitialEntry = "/account-pool/upstream-accounts") {
  act(() => {
    root?.render(
      <I18nProvider>
        <SystemNotificationProvider>
          <MemoryRouter initialEntries={[initialEntry]}>
            <Routes>
              <Route path="/account-pool/upstream-accounts" element={<UpstreamAccountsPage />} />
            </Routes>
          </MemoryRouter>
        </SystemNotificationProvider>
      </I18nProvider>,
    );
  });
}
function remount(initialEntry: InitialEntry = "/account-pool/upstream-accounts") {
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
  if (expected == null) {
    expect(hookMocks.useUpstreamAccounts.mock.calls).toContainEqual([expected]);
    return;
  }
  expect(hookMocks.useUpstreamAccounts.mock.calls).toContainEqual([
    expect.objectContaining({ ...expected, kind: "oauth_codex" }),
  ]);
}
function findButton(pattern: RegExp) {
  return Array.from(document.body.querySelectorAll("button")).find((candidate) =>
    pattern.test(candidate.textContent || candidate.getAttribute("aria-label") || ""),
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
  return Array.from(document.body.querySelectorAll(".fixed")).find((candidate) =>
    pattern.test(candidate.textContent || ""),
  ) as HTMLElement | undefined;
}
async function flushAsync() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}
async function waitForAssertion(check: () => void, attempts = 20) {
  let lastError: unknown = null;
  for (let index = 0; index < attempts; index += 1) {
    try {
      check();
      return;
    } catch (error) {
      lastError = error;
      await flushAsync();
    }
  }
  throw lastError;
}
async function flushTimers() {
  await act(async () => {
    await new Promise((resolve) => window.setTimeout(resolve, 0));
  });
}
function setCompactViewportMatch(matches: boolean) {
  compactViewportMatches = matches;
}
function setInputValue(selector: string, value: string) {
  const input = document.body.querySelector(selector);
  if (!(input instanceof HTMLInputElement || input instanceof HTMLTextAreaElement)) {
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
function setFieldValue(input: HTMLInputElement | HTMLTextAreaElement, value: string) {
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
  const option = Array.from(document.body.querySelectorAll("[cmdk-item]")).find((candidate) =>
    (candidate.textContent || "").includes(value),
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
        candidate.textContent || candidate.getAttribute("aria-label") || candidate.title || "",
      ),
  );
  if (!(button instanceof HTMLButtonElement)) throw new Error(`missing button: ${matcher}`);
  pressButton(button);
  return button;
}
function clickTab(matcher: RegExp) {
  const tab = Array.from(document.body.querySelectorAll('[role="tab"]')).find(
    (candidate) =>
      candidate instanceof HTMLButtonElement &&
      matcher.test(candidate.textContent || candidate.getAttribute("aria-label") || ""),
  );
  if (!(tab instanceof HTMLButtonElement)) {
    throw new Error(`missing tab: ${matcher}`);
  }
  pressButton(tab);
  return tab;
}
function expandLoginHealthDetails() {
  const summary = document.body.querySelector(
    '[data-testid="upstream-account-login-health-summary"] details > summary',
  );
  if (!(summary instanceof HTMLElement)) {
    throw new Error("missing login health diagnostic details");
  }
  act(() => {
    const details = summary.closest("details");
    if (!(details instanceof HTMLDetailsElement)) {
      throw new Error("missing login health details container");
    }
    details.open = true;
  });
}
function clickDrawerBackdrop() {
  const overlay =
    document.body.querySelector(".drawer-shell")?.parentElement?.previousElementSibling;
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
  const overlay = gutter?.previousElementSibling;
  if (!(gutter instanceof HTMLElement) || !(overlay instanceof HTMLElement)) {
    throw new Error("missing drawer gutter");
  }
  act(() => {
    overlay.dispatchEvent(new MouseEvent("click", { bubbles: true }));
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
  const checkbox = Array.from(document.body.querySelectorAll('input[type="checkbox"]')).find(
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
  const trigger = Array.from(document.body.querySelectorAll('button[role="combobox"]')).find(
    (candidate) =>
      candidate instanceof HTMLButtonElement &&
      matcher.test(candidate.getAttribute("aria-label") || candidate.textContent || ""),
  );
  if (!(trigger instanceof HTMLButtonElement)) {
    throw new Error(`missing combobox: ${matcher}`);
  }
  pressButton(trigger);
  return trigger;
}
function clickCommandItem(matcher: RegExp) {
  const item = Array.from(document.body.querySelectorAll("[cmdk-item]")).find(
    (candidate) => candidate instanceof HTMLElement && matcher.test(candidate.textContent || ""),
  );
  if (!(item instanceof HTMLElement)) {
    throw new Error(`missing command item: ${matcher}`);
  }
  act(() => {
    item.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  return item;
}
function renderedInvocationAccountNames() {
  return Array.from(document.body.querySelectorAll('[data-testid="invocation-account-name"]'))
    .map((candidate) => candidate.textContent?.trim() ?? "")
    .filter((value) => value.length > 0);
}
function _clickSelectOption(matcher: RegExp) {
  const option = Array.from(document.body.querySelectorAll('[role="option"]')).find(
    (candidate) => candidate instanceof HTMLElement && matcher.test(candidate.textContent || ""),
  );
  if (!(option instanceof HTMLElement)) {
    throw new Error(`missing select option: ${matcher}`);
  }
  act(() => {
    option.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  return option;
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
  allowCutOut: true,
  allowCutIn: true,
  sourceTagIds: [],
  sourceTagNames: [],
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

export {
  _clickSelectOption,
  apiMocks,
  buildBulkSyncCounts,
  buildBulkSyncJobResponse,
  buildBulkSyncSnapshot,
  buildBulkSyncSnapshotEvent,
  clickButton,
  clickCheckboxByLabel,
  clickCombobox,
  clickCommandItem,
  clickDrawerBackdrop,
  clickDrawerGutter,
  clickFirstRosterRow,
  clickTab,
  compactViewportMatches,
  defaultEffectiveRoutingRule,
  defaultPoolTags,
  deferred,
  expandLoginHealthDetails,
  expectRosterHookQuery,
  findButton,
  findExactTextElements,
  findFixedContainerByText,
  flushAsync,
  flushTimers,
  hookMocks,
  host,
  LOCALE_STORAGE_KEY,
  MockBulkSyncEventSource,
  navigateMock,
  pressButton,
  readStoredUpstreamFilters,
  remount,
  render,
  renderedInvocationAccountNames,
  renderFlatForLegacySuites,
  rerender,
  root,
  setComboboxValue,
  setCompactViewportMatch,
  setFieldValue,
  setHost,
  setInputValue,
  setRoot,
  sseMocks,
  storage,
  UPSTREAM_ACCOUNTS_FILTER_STORAGE_KEY,
  virtualizerMocks,
  waitForAssertion,
  writeStoredUpstreamFilters,
};
