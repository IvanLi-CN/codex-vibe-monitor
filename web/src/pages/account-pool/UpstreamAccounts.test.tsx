/* eslint-disable @typescript-eslint/ban-ts-comment, @typescript-eslint/no-unused-vars */
// @ts-nocheck

/** @vitest-environment jsdom */
import * as React from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import {
  createMemoryRouter,
  type InitialEntry,
  MemoryRouter,
  Route,
  RouterProvider,
  Routes,
} from "react-router-dom";
import ts from "typescript";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { SystemNotificationProvider } from "../../components/ui/system-notifications";
import { COMPACT_VIEWPORT_MEDIA_QUERY } from "../../hooks/useCompactViewport";
import { I18nProvider } from "../../i18n";
import type { BroadcastPayload, EffectiveRoutingRule, TagSummary } from "../../lib/api";
import { ApiRequestError } from "../../lib/api";
import { ThemeProvider } from "../../theme/context";
import UpstreamAccountsPage, { SharedUpstreamAccountDetailDrawer } from "./UpstreamAccounts";
import suite5 from "./UpstreamAccounts.api-key-details.txt?raw";
import suite6 from "./UpstreamAccounts.delete-confirmation.txt?raw";
import suite2 from "./UpstreamAccounts.duplicates.txt?raw";
import suite7 from "./UpstreamAccounts.edit-drafts.txt?raw";
import suite4 from "./UpstreamAccounts.oauth-recovery.txt?raw";
import suite1 from "./UpstreamAccounts.roster-freshness.txt?raw";
import suite3 from "./UpstreamAccounts.sync-state-isolation.txt?raw";

type UpstreamAccountsHookValue = ReturnType<
  typeof import("../../hooks/useUpstreamAccounts").useUpstreamAccounts
>;

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

describe("UpstreamAccountsPage detail route tab query", () => {
  it("opens the shared drawer on the routing tab when the query requests routing", async () => {
    mockAccountsPage();
    render("/account-pool/upstream-accounts?upstreamAccountId=5&upstreamAccountTab=routing");
    await flushAsync();

    expect(document.body.textContent ?? "").toMatch(/最终生效规则|Effective routing rule/);
    expect(document.body.textContent ?? "").toMatch(/字段来源明细|Field source breakdown/);
  });

  it("defaults to the overview tab when only the account id is present", async () => {
    mockAccountsPage();
    render("/account-pool/upstream-accounts?upstreamAccountId=5");
    await flushAsync();

    const overviewTab = Array.from(document.body.querySelectorAll('button[role="tab"]')).find(
      (candidate) => /概览|overview/i.test(candidate.textContent ?? ""),
    );
    if (!(overviewTab instanceof HTMLButtonElement)) {
      throw new Error("missing overview tab");
    }

    expect(overviewTab.getAttribute("aria-selected")).toBe("true");
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
  expect(hookMocks.useUpstreamAccounts.mock.calls).toContainEqual([expected]);
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
  saveGroupNote?: ReturnType<typeof vi.fn>;
  deleteGroupNote?: ReturnType<typeof vi.fn>;
  loadDetail?: ReturnType<typeof vi.fn>;
  groups?: Array<Record<string, unknown>>;
  routing?: {
    writesEnabled: boolean;
    apiKeyConfigured: boolean;
    maskedApiKey: string | null;
    timeouts: {
      responsesFirstByteTimeoutSecs: number;
      compactFirstByteTimeoutSecs: number;
      imageFirstByteTimeoutSecs: number;
      responsesStreamTimeoutSecs: number;
      compactStreamTimeoutSecs: number;
    };
  } | null;
  item?: Record<string, unknown>;
  selectedSummary?: Record<string, unknown>;
  detail?: Record<string, unknown>;
}) {
  const saveRouting = options?.saveRouting ?? vi.fn();
  const loadDetail = options?.loadDetail ?? vi.fn();
  const groups = (
    options?.groups ?? [
      {
        groupName: "prod",
        accountCount: 2,
        note: "prod note",
        boundProxyKeys: ["jp-edge-01"],
        nodeShuntEnabled: false,
        singleAccountRotationEnabled: false,
        upstream429RetryEnabled: false,
        upstream429MaxRetries: 0,
      },
    ]
  ).map((group) => ({
    ...group,
    accountCount: typeof group.accountCount === "number" ? group.accountCount : 0,
    boundProxyKeys: Array.isArray(group.boundProxyKeys)
      ? group.boundProxyKeys.map((value) => String(value))
      : [],
    nodeShuntEnabled: group.nodeShuntEnabled === true,
    singleAccountRotationEnabled: group.singleAccountRotationEnabled === true,
    upstream429RetryEnabled: group.upstream429RetryEnabled === true,
    upstream429MaxRetries:
      typeof group.upstream429MaxRetries === "number" ? group.upstream429MaxRetries : 0,
    concurrencyLimit: typeof group.concurrencyLimit === "number" ? group.concurrencyLimit : 0,
  }));
  const saveGroupNote: ReturnType<typeof vi.fn> =
    options?.saveGroupNote ??
    vi.fn().mockImplementation(async (groupName: string, payload: Record<string, unknown>) => {
      const normalizedGroupName = groupName.trim();
      const nextSummary = {
        groupName: normalizedGroupName,
        accountCount:
          groups.find((group) => group.groupName === normalizedGroupName)?.accountCount ?? 0,
        note:
          typeof payload.note === "string" && payload.note.trim().length > 0 ? payload.note : null,
        boundProxyKeys: Array.isArray(payload.boundProxyKeys)
          ? payload.boundProxyKeys.map((value) => String(value))
          : [],
        nodeShuntEnabled: payload.nodeShuntEnabled === true,
        singleAccountRotationEnabled: payload.singleAccountRotationEnabled === true,
        upstream429RetryEnabled: payload.upstream429RetryEnabled === true,
        upstream429MaxRetries:
          typeof payload.upstream429MaxRetries === "number" ? payload.upstream429MaxRetries : 0,
        concurrencyLimit:
          typeof payload.concurrencyLimit === "number" ? payload.concurrencyLimit : 0,
      };
      const existingIndex = groups.findIndex((group) => group.groupName === normalizedGroupName);
      if (existingIndex >= 0) {
        groups.splice(existingIndex, 1, nextSummary);
      } else {
        groups.push(nextSummary);
      }
      return nextSummary;
    });
  const deleteGroupNote: ReturnType<typeof vi.fn> =
    options?.deleteGroupNote ??
    vi.fn().mockImplementation(async (groupName: string) => {
      const normalizedGroupName = groupName.trim();
      const existingIndex = groups.findIndex((group) => group.groupName === normalizedGroupName);
      if (existingIndex >= 0) {
        groups.splice(existingIndex, 1);
      }
    });
  const compactSupport = {
    status: "unsupported" as const,
    observedAt: "2026-03-16T02:08:00.000Z",
    reason: "No available channel for compact model gpt-5.4-openai-compact",
  };
  const routingTimeouts = {
    responsesFirstByteTimeoutSecs: 120,
    compactFirstByteTimeoutSecs: 300,
    imageFirstByteTimeoutSecs: 300,
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
        attemptId: "4V7MYPJG",
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
    isDetailRecentActionsHydrated: false,
    listError: null,
    detailError: null,
    error: null,
    selectAccount: vi.fn(),
    refresh: vi.fn(),
    loadDetail,
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
    saveGroupNote,
    deleteGroupNote,
    runBulkAction: vi.fn(),
    startBulkSyncJob: vi.fn(),
    getBulkSyncJob: vi.fn(),
    stopBulkSyncJob: vi.fn(),
    runSync: vi.fn(),
    removeAccount: vi.fn(),
    forwardProxyNodes: [],
    forwardProxyCatalogState: {
      kind: "loaded",
      freshness: "fresh",
    },
    groups,
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
  return {
    saveRouting,
    compactSupport,
    routingTimeouts,
    saveGroupNote,
    deleteGroupNote,
    loadDetail,
  };
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
  saveGroupNote?: ReturnType<typeof vi.fn>;
  groups?: Array<Record<string, unknown>>;
}) {
  const refresh = options?.refresh ?? vi.fn();
  const saveGroupNote = options?.saveGroupNote ?? vi.fn();
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
        enableStatus: "enabled",
        workStatus: "working",
        healthStatus: "normal",
        syncState: "idle",
        activeConversationCount: 2,
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
        enableStatus: "enabled",
        workStatus: "idle",
        healthStatus: "normal",
        syncState: "idle",
        activeConversationCount: 0,
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
    saveGroupNote,
    deleteGroupNote: vi.fn(),
    runBulkAction: vi.fn(),
    startBulkSyncJob: vi.fn(),
    getBulkSyncJob: vi.fn(),
    stopBulkSyncJob: vi.fn(),
    runSync: vi.fn(),
    removeAccount: vi.fn(),
    groups: options?.groups ?? [
      {
        groupName: "prod",
        accountCount: 2,
        note: "prod note",
        boundProxyKeys: ["jp-edge-01"],
        nodeShuntEnabled: false,
        upstream429RetryEnabled: false,
        upstream429MaxRetries: 0,
      },
    ],
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
  render: renderFlatForLegacySuites,
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
  host: {
    get: () => host,
    set: (value) => {
      host = value as typeof host;
    },
  },
  root: {
    get: () => root,
    set: (value) => {
      root = value as typeof root;
    },
  },
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

describe("UpstreamAccountsPage grouped roster toggle", () => {
  it("defaults to grid view with grid, grouped, flat selector order", async () => {
    mockRosterFreshnessPage();
    render();
    await act(async () => {
      await Promise.resolve();
    });

    const tabs = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []);
    expect(tabs[0]?.textContent ?? "").toMatch(/grid|网格/i);
    expect(tabs[1]?.textContent ?? "").toMatch(/grouped|分组/i);
    expect(tabs[2]?.textContent ?? "").toMatch(/flat|平铺/i);
    expect(tabs[0]?.getAttribute("aria-selected")).toBe("true");
    expectRosterHookQuery({ includeAll: true });
    expect(host?.querySelector('[data-testid="upstream-accounts-grouped-roster"]')).toBeTruthy();
    expect(host?.querySelector('[data-testid="upstream-accounts-pagination-footer"]')).toBeNull();
  });

  it("switches to grouped view and hides the pagination footer", async () => {
    mockRosterFreshnessPage();
    render();
    await act(async () => {
      await Promise.resolve();
    });

    const groupedToggle = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (candidate) => /grouped|分组/i.test(candidate.textContent ?? ""),
    );
    expect(groupedToggle).toBeTruthy();

    act(() => {
      groupedToggle?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    const groupedRoster = host?.querySelector(
      '[data-testid="upstream-accounts-grouped-roster"]',
    ) as HTMLElement | null;
    expect(groupedRoster).toBeTruthy();
    expect(groupedRoster?.className ?? "").not.toContain("overflow-auto");
    expect(
      Array.from(host?.querySelectorAll('input[type="checkbox"]') ?? []).find((candidate) =>
        /select filtered accounts|选择筛选结果/i.test(candidate.getAttribute("aria-label") ?? ""),
      ),
    ).toBeTruthy();
    expect(host?.querySelector('[data-testid="upstream-accounts-pagination-footer"]')).toBeNull();
  });

  it("switches to grid view without bulk selection controls", async () => {
    mockRosterFreshnessPage();
    render();
    await act(async () => {
      await Promise.resolve();
    });

    const gridToggle = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (candidate) => /grid|网格/i.test(candidate.textContent ?? ""),
    );
    expect(gridToggle).toBeTruthy();

    act(() => {
      gridToggle?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    const groupedRoster = host?.querySelector(
      '[data-testid="upstream-accounts-grouped-roster"]',
    ) as HTMLElement | null;
    expect(groupedRoster).toBeTruthy();
    expect(gridToggle?.getAttribute("aria-selected")).toBe("true");
    expect(groupedRoster?.className ?? "").not.toContain("overflow-auto");
    expect(
      Array.from(host?.querySelectorAll('input[type="checkbox"]') ?? []).find((candidate) =>
        /select filtered accounts|选择筛选结果/i.test(candidate.getAttribute("aria-label") ?? ""),
      ),
    ).toBeFalsy();
    expect(groupedRoster?.textContent ?? "").toMatch(/Working 2|工作中 2|工作 2/i);
    expect(host?.querySelector('[data-testid="upstream-accounts-pagination-footer"]')).toBeNull();
  });

  it("blocks grouped roster interactions while the include-all query is still switching", async () => {
    mockRosterFreshnessPage({
      listState: {
        queryKey: '{"includeAll":true}',
        dataQueryKey: '{"page":2,"pageSize":20}',
        freshness: "stale",
        loadingState: "switching",
        status: "ready",
        hasCurrentQueryData: true,
        isPending: true,
      },
    });
    render();
    await act(async () => {
      await Promise.resolve();
    });

    const groupedToggle = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (candidate) => /grouped|分组/i.test(candidate.textContent ?? ""),
    );
    expect(groupedToggle).toBeTruthy();

    act(() => {
      groupedToggle?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(host?.querySelector('[data-testid="upstream-accounts-grouped-loading"]')).toBeTruthy();
    expect(host?.querySelector('[data-testid="upstream-accounts-grouped-roster"]')).toBeNull();
    expect(
      Array.from(host?.querySelectorAll('input[type="checkbox"]') ?? []).find((candidate) =>
        /select filtered accounts|选择筛选结果/i.test(candidate.getAttribute("aria-label") ?? ""),
      ),
    ).toBeFalsy();
  });

  it("blocks flat roster interactions while switching away from grouped include-all data", async () => {
    mockRosterFreshnessPage({
      listState: {
        queryKey: '{"page":2,"pageSize":20}',
        dataQueryKey: '{"includeAll":true}',
        freshness: "stale",
        loadingState: "switching",
        status: "ready",
        hasCurrentQueryData: true,
        isPending: true,
      },
    });
    render();
    await act(async () => {
      await Promise.resolve();
    });

    const flatToggle = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (candidate) => /flat|平铺/i.test(candidate.textContent ?? ""),
    );
    expect(flatToggle).toBeTruthy();
    act(() => {
      flatToggle?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(host?.querySelector('[data-testid="upstream-accounts-table-loading"]')).toBeTruthy();
    expect(host?.querySelector('[data-testid="upstream-accounts-pagination-footer"]')).toBeTruthy();
    expect(host?.textContent ?? "").not.toContain("Existing OAuth");
    expect(host?.textContent ?? "").not.toContain("Another OAuth");
  });

  it("preserves the flat page selection when toggling grouped view", async () => {
    const items = [
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
    ];
    const sharedCallbacks: Pick<
      UpstreamAccountsHookValue,
      | "selectAccount"
      | "refresh"
      | "loadDetail"
      | "beginOauthLogin"
      | "beginRelogin"
      | "beginOauthMailboxSession"
      | "beginOauthMailboxSessionForAddress"
      | "getOauthMailboxStatuses"
      | "removeOauthMailboxSession"
      | "getLoginSession"
      | "completeOauthLogin"
      | "createApiKeyAccount"
      | "saveAccount"
      | "saveRouting"
      | "saveGroupNote"
      | "deleteGroupNote"
      | "runBulkAction"
      | "startBulkSyncJob"
      | "getBulkSyncJob"
      | "stopBulkSyncJob"
      | "runSync"
      | "removeAccount"
    > = {
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
      deleteGroupNote: vi.fn(),
      runBulkAction: vi.fn(),
      startBulkSyncJob: vi.fn(),
      getBulkSyncJob: vi.fn(),
      stopBulkSyncJob: vi.fn(),
      runSync: vi.fn(),
      removeAccount: vi.fn(),
    };
    const sharedRouting: UpstreamAccountsHookValue["routing"] = {
      writesEnabled: true,
      apiKeyConfigured: false,
      maskedApiKey: null,
    };
    const sharedBase: Omit<UpstreamAccountsHookValue, "page" | "pageSize" | "listState"> = {
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
      forwardProxyNodes: [],
      forwardProxyCatalogState: {
        kind: "ready-empty",
        freshness: "fresh",
        isPending: false,
        hasNodes: false,
      },
      missingDetailAccountId: null,
      isWindowUsagePending: false,
      ...sharedCallbacks,
    };
    const flatPage1: UpstreamAccountsHookValue = {
      ...sharedBase,
      page: 1,
      pageSize: 20,
      listState: {
        queryKey: '{"page":1,"pageSize":20}',
        dataQueryKey: '{"page":1,"pageSize":20}',
        freshness: "fresh",
        loadingState: "idle",
        status: "ready",
        hasCurrentQueryData: true,
        isPending: false,
      },
    };
    const flatPage2: UpstreamAccountsHookValue = {
      ...sharedBase,
      page: 2,
      pageSize: 20,
      listState: {
        queryKey: '{"page":2,"pageSize":20}',
        dataQueryKey: '{"page":2,"pageSize":20}',
        freshness: "fresh",
        loadingState: "idle",
        status: "ready",
        hasCurrentQueryData: true,
        isPending: false,
      },
    };
    const groupedAll: UpstreamAccountsHookValue = {
      ...sharedBase,
      page: 1,
      pageSize: 45,
      listState: {
        queryKey: '{"includeAll":true}',
        dataQueryKey: '{"includeAll":true}',
        freshness: "fresh",
        loadingState: "idle",
        status: "ready",
        hasCurrentQueryData: true,
        isPending: false,
      },
    };
    hookMocks.useUpstreamAccounts.mockImplementation((query) => {
      if (query?.includeAll) return groupedAll;
      if (query?.page === 2) return flatPage2;
      return flatPage1;
    });

    render();
    await act(async () => {
      await Promise.resolve();
    });

    const initialFlatToggle = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (candidate) => /flat|平铺/i.test(candidate.textContent ?? ""),
    );
    expect(initialFlatToggle).toBeTruthy();
    act(() => {
      initialFlatToggle?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    const nextButton = findButton(/next|下一页/i);
    expect(nextButton).toBeTruthy();
    act(() => {
      nextButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expectRosterHookQuery({ page: 2, pageSize: 20 });

    const groupedToggle = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (candidate) => /grouped|分组/i.test(candidate.textContent ?? ""),
    );
    const flatToggle = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (candidate) => /flat|平铺/i.test(candidate.textContent ?? ""),
    );
    expect(groupedToggle).toBeTruthy();
    expect(flatToggle).toBeTruthy();

    act(() => {
      groupedToggle?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expectRosterHookQuery({ includeAll: true });

    act(() => {
      flatToggle?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expectRosterHookQuery({ page: 2, pageSize: 20 });
  });

  it("opens the shared group settings dialog from the grouped summary action", async () => {
    const saveGroupNote = vi.fn();
    mockRosterFreshnessPage({
      saveGroupNote,
      groups: [
        {
          groupName: "  prod  ",
          note: "Production routing group",
          boundProxyKeys: ["jp-edge-01"],
          concurrencyLimit: 3,
          nodeShuntEnabled: true,
          upstream429RetryEnabled: true,
          upstream429MaxRetries: 2,
        },
      ],
    });
    render();
    await act(async () => {
      await Promise.resolve();
    });

    const groupedToggle = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (candidate) => /grouped|分组/i.test(candidate.textContent ?? ""),
    );
    expect(groupedToggle).toBeTruthy();

    act(() => {
      groupedToggle?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    const settingsButton = Array.from(host?.querySelectorAll("button") ?? []).find((candidate) =>
      /edit group settings|编辑分组设置/i.test(
        candidate.getAttribute("aria-label") ?? candidate.textContent ?? "",
      ),
    );
    expect(settingsButton).toBeTruthy();

    act(() => {
      settingsButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    await act(async () => {
      await Promise.resolve();
    });

    const dialogs = Array.from(host?.ownerDocument.querySelectorAll('[role="dialog"]') ?? []);
    const groupSettingsDialog = dialogs.find((candidate) =>
      /group settings|分组设置/i.test(candidate.textContent ?? ""),
    );
    expect(groupSettingsDialog).toBeTruthy();
    expect(groupSettingsDialog?.textContent ?? "").toContain("prod");
    expect(groupSettingsDialog?.textContent ?? "").toContain("JP Edge 01");
    expect(saveGroupNote).not.toHaveBeenCalled();
  });

  it("treats grouped summary actions as existing groups when the catalog returns them", async () => {
    const saveGroupNote = vi.fn();
    mockRosterFreshnessPage({
      saveGroupNote,
      groups: [
        {
          groupName: "prod",
          accountCount: 2,
          note: "Production routing group",
          boundProxyKeys: ["jp-edge-01"],
          nodeShuntEnabled: false,
          upstream429RetryEnabled: false,
          upstream429MaxRetries: 0,
        },
      ],
    });
    render();
    await act(async () => {
      await Promise.resolve();
    });

    const groupedToggle = Array.from(host?.querySelectorAll('button[role="tab"]') ?? []).find(
      (candidate) => /grouped|分组/i.test(candidate.textContent ?? ""),
    );
    expect(groupedToggle).toBeTruthy();

    act(() => {
      groupedToggle?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    const settingsButton = Array.from(host?.querySelectorAll("button") ?? []).find((candidate) =>
      /edit group settings|编辑分组设置/i.test(
        candidate.getAttribute("aria-label") ?? candidate.textContent ?? "",
      ),
    );
    expect(settingsButton).toBeTruthy();

    act(() => {
      settingsButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    await act(async () => {
      await Promise.resolve();
    });

    const dialogs = Array.from(host?.ownerDocument.querySelectorAll('[role="dialog"]') ?? []);
    const groupSettingsDialog = dialogs.find((candidate) =>
      /group settings|分组设置/i.test(candidate.textContent ?? ""),
    );
    expect(groupSettingsDialog).toBeTruthy();
    expect(groupSettingsDialog?.textContent ?? "").toContain("already exists");
    expect(groupSettingsDialog?.textContent ?? "").not.toContain(
      "creates its shared settings in advance",
    );

    const saveButton = Array.from(groupSettingsDialog?.querySelectorAll("button") ?? []).find(
      (candidate) => /save|保存/i.test(candidate.textContent ?? ""),
    );
    expect(saveButton).toBeTruthy();

    act(() => {
      saveButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    await act(async () => {
      await Promise.resolve();
    });

    expect(saveGroupNote).toHaveBeenCalledTimes(1);
  });

  it("consumes a named presetGroupFilter from location state and clears the one-shot navigation state", async () => {
    mockRosterFreshnessPage();
    writeStoredUpstreamFilters({
      workStatus: [],
      enableStatus: [],
      healthStatus: [],
      tagIds: [],
      groupFilter: {
        mode: "search",
        query: "stale-group",
      },
    });
    render({
      pathname: "/account-pool/upstream-accounts",
      state: {
        presetGroupFilter: {
          mode: "exact",
          query: "  prod  ",
        },
      },
    });

    await flushAsync();
    await flushAsync();

    expect(hookMocks.useUpstreamAccounts.mock.calls[0]?.[0]).toEqual({
      includeAll: true,
      groupExact: ["prod"],
    });
    expectRosterHookQuery({
      includeAll: true,
      groupExact: ["prod"],
    });
    expect(hookMocks.useUpstreamAccounts.mock.calls).not.toContainEqual([
      {
        page: 1,
        pageSize: 20,
        groupExact: ["stale-group"],
      },
    ]);
    expect(readStoredUpstreamFilters()?.groupFilters).toEqual(["stale-group"]);
    expect(navigateMock).toHaveBeenCalledWith(
      {
        pathname: "/account-pool/upstream-accounts",
        search: "",
      },
      {
        replace: true,
        state: null,
      },
    );
  });

  it("consumes an ungrouped presetGroupFilter from location state and resets the roster to page 1", async () => {
    mockRosterFreshnessPage();
    writeStoredUpstreamFilters({
      workStatus: [],
      enableStatus: [],
      healthStatus: [],
      tagIds: [],
      groupFilter: {
        mode: "search",
        query: "stale-group",
      },
    });
    render({
      pathname: "/account-pool/upstream-accounts",
      state: {
        presetGroupFilter: {
          mode: "ungrouped",
          query: "",
        },
      },
    });

    await flushAsync();
    await flushAsync();

    expectRosterHookQuery({
      includeAll: true,
      groupExact: ["未分组"],
    });
    expect(readStoredUpstreamFilters()?.groupFilters).toEqual(["stale-group"]);
    expect(navigateMock).toHaveBeenCalledWith(
      {
        pathname: "/account-pool/upstream-accounts",
        search: "",
      },
      {
        replace: true,
        state: null,
      },
    );
  });

  it("keeps detail-drawer roster hydration disabled until a roster-dependent tab opens", async () => {
    mockRosterFreshnessPage();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() => {
      root?.render(
        <I18nProvider>
          <SystemNotificationProvider>
            <MemoryRouter>
              <SharedUpstreamAccountDetailDrawer open accountId={5} onClose={vi.fn()} />
            </MemoryRouter>
          </SystemNotificationProvider>
        </I18nProvider>,
      );
    });

    await flushAsync();
    await flushAsync();

    expect(hookMocks.useUpstreamAccounts.mock.calls[0]?.[0]).toBeNull();
    expect(hookMocks.useUpstreamStickyConversations.mock.calls.at(-1)?.[1]).toEqual({
      mode: "count",
      limit: 50,
    });
    expect(hookMocks.useUpstreamStickyConversations.mock.calls.at(-1)?.[2]).toBe(false);

    act(() => {
      root?.render(
        <I18nProvider>
          <SystemNotificationProvider>
            <MemoryRouter>
              <SharedUpstreamAccountDetailDrawer
                open={false}
                accountId={5}
                initialTab="routing"
                onClose={vi.fn()}
              />
            </MemoryRouter>
          </SystemNotificationProvider>
        </I18nProvider>,
      );
    });
    await flushAsync();
    hookMocks.useUpstreamAccounts.mockClear();
    hookMocks.useUpstreamStickyConversations.mockClear();
    act(() => {
      root?.render(
        <I18nProvider>
          <SystemNotificationProvider>
            <MemoryRouter>
              <SharedUpstreamAccountDetailDrawer
                open
                accountId={5}
                initialTab="routing"
                onClose={vi.fn()}
              />
            </MemoryRouter>
          </SystemNotificationProvider>
        </I18nProvider>,
      );
    });
    await flushAsync();

    expect(hookMocks.useUpstreamAccounts.mock.calls).toContainEqual([
      undefined,
      {
        allowSelectionOutsideList: true,
        fallbackToFirstItem: false,
      },
    ]);
  });

  it("renders the detail delete confirmation as a sheet on compact viewports", async () => {
    setCompactViewportMatch(true);
    mockAccountsPage();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() => {
      root?.render(
        <I18nProvider>
          <SystemNotificationProvider>
            <MemoryRouter>
              <SharedUpstreamAccountDetailDrawer
                open
                accountId={5}
                presentation="page"
                onClose={vi.fn()}
              />
            </MemoryRouter>
          </SystemNotificationProvider>
        </I18nProvider>,
      );
    });

    await flushAsync();
    clickButton(/^delete$|^删除$/i);
    await flushAsync();

    const confirmDialog = document.body.querySelector('.dialog-surface[role="alertdialog"]');
    expect(confirmDialog).not.toBeNull();
    expect(confirmDialog?.textContent ?? "").toMatch(/Existing OAuth/);
  });

  it.skip("subscribes the records tab fetch lifecycle to the selected upstream account and reconciles on SSE open", async () => {
    mockAccountsPage();
    apiMocks.fetchInvocationRecords
      .mockResolvedValueOnce({
        snapshotId: 42,
        total: 1,
        page: 1,
        pageSize: 50,
        records: [
          {
            id: 1,
            invokeId: "invoke-live",
            occurredAt: "2026-03-16T02:05:00.000Z",
            createdAt: "2026-03-16T02:05:00.000Z",
            status: "running",
            model: "gpt-5.4",
            upstreamAccountId: 5,
            upstreamAccountName: "Existing OAuth",
            routeMode: "pool",
          },
        ],
      })
      .mockResolvedValueOnce({
        snapshotId: 84,
        total: 51,
        page: 1,
        pageSize: 50,
        records: [
          {
            id: 2,
            invokeId: "invoke-new",
            occurredAt: "2026-03-16T02:06:00.000Z",
            createdAt: "2026-03-16T02:06:00.000Z",
            status: "success",
            model: "gpt-5.4",
            upstreamAccountId: 5,
            upstreamAccountName: "Existing OAuth",
            routeMode: "pool",
          },
          {
            id: 1,
            invokeId: "invoke-live",
            occurredAt: "2026-03-16T02:05:00.000Z",
            createdAt: "2026-03-16T02:05:00.000Z",
            status: "success",
            model: "gpt-5.4",
            totalTokens: 777,
            upstreamAccountId: 5,
            upstreamAccountName: "Existing OAuth",
            routeMode: "pool",
          },
        ],
      });

    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() => {
      root?.render(
        <ThemeProvider>
          <I18nProvider>
            <SystemNotificationProvider>
              <MemoryRouter>
                <SharedUpstreamAccountDetailDrawer
                  open
                  accountId={5}
                  initialTab="records"
                  onClose={vi.fn()}
                />
              </MemoryRouter>
            </SystemNotificationProvider>
          </I18nProvider>
        </ThemeProvider>,
      );
    });

    await waitForAssertion(() => {
      expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledTimes(1);
    });
    expect(apiMocks.fetchInvocationRecords.mock.calls[0]?.[0]).toMatchObject({
      upstreamAccountId: 5,
      page: 1,
      pageSize: 50,
      sortBy: "occurredAt",
      sortOrder: "desc",
    });

    act(() => {
      sseMocks.onMessage?.({
        type: "records",
        records: [
          {
            id: 99,
            invokeId: "invoke-other-account",
            occurredAt: "2026-03-16T02:07:00.000Z",
            createdAt: "2026-03-16T02:07:00.000Z",
            status: "success",
            model: "gpt-5.4",
            upstreamAccountId: 9,
            upstreamAccountName: "Another OAuth",
            routeMode: "pool",
          },
          {
            id: 1,
            invokeId: "invoke-live",
            occurredAt: "2026-03-16T02:05:00.000Z",
            createdAt: "2026-03-16T02:05:00.000Z",
            status: "success",
            model: "gpt-5.4",
            totalTokens: 777,
            upstreamAccountId: 5,
            upstreamAccountName: "Existing OAuth",
            routeMode: "pool",
          },
          {
            id: 2,
            invokeId: "invoke-new",
            occurredAt: "2026-03-16T02:06:00.000Z",
            createdAt: "2026-03-16T02:06:00.000Z",
            status: "success",
            model: "gpt-5.4",
            upstreamAccountId: 5,
            upstreamAccountName: "Existing OAuth",
            routeMode: "pool",
          },
        ],
      });
    });

    await flushAsync();
    expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledTimes(1);

    act(() => {
      sseMocks.onOpen?.();
    });
    await act(async () => {
      await Promise.resolve();
    });
  });

  it("hydrates recent actions only after switching to the health events tab", async () => {
    const loadDetail = vi.fn();
    mockAccountsPage({
      detail: {
        recentActions: [],
      },
      loadDetail,
    });

    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() => {
      root?.render(
        <ThemeProvider>
          <I18nProvider>
            <SystemNotificationProvider>
              <MemoryRouter>
                <SharedUpstreamAccountDetailDrawer
                  open
                  accountId={5}
                  initialTab="overview"
                  onClose={vi.fn()}
                />
              </MemoryRouter>
            </SystemNotificationProvider>
          </I18nProvider>
        </ThemeProvider>,
      );
    });

    await flushAsync();
    expect(loadDetail).not.toHaveBeenCalled();

    clickTab(/健康与事件|health/i);
    await flushAsync();

    expect(loadDetail).toHaveBeenCalledWith(5, {
      silent: true,
      includeRecentActions: true,
    });
  });

  it("labels a linked health event with its upstream attempt id", async () => {
    mockAccountsPage();

    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() => {
      root?.render(
        <ThemeProvider>
          <I18nProvider>
            <SystemNotificationProvider>
              <MemoryRouter>
                <SharedUpstreamAccountDetailDrawer
                  open
                  accountId={5}
                  initialTab="healthEvents"
                  onClose={vi.fn()}
                />
              </MemoryRouter>
            </SystemNotificationProvider>
          </I18nProvider>
        </ThemeProvider>,
      );
    });

    await flushAsync();
    expect(document.body.textContent).toMatch(/上游尝试 ID|Upstream attempt ID/);
    expect(document.body.textContent).toContain("4V7MYPJG");
    expect(document.body.textContent).not.toMatch(/请求 ID: invk_action_001/);
  });

  it("opens the blocked-binding working conversation filter from a health event", async () => {
    mockAccountsPage({
      detail: {
        recentActions: [
          {
            id: 71,
            occurredAt: "2026-03-16T02:06:00.000Z",
            action: "route_hard_unavailable",
            source: "call",
            reasonCode: "pool_assigned_account_blocked",
            reasonMessage: "This conversation is pinned to one blocked account",
            httpStatus: 502,
            failureKind: "pool_assigned_account_blocked",
            invokeId: "invk_action_001",
            attemptId: "4V7MYPJG",
            stickyKey: "sticky-dup-001",
            createdAt: "2026-03-16T02:06:00.000Z",
            blockedBinding: {
              upstreamAccountId: 2890,
              constraintSource: "encryptedSessionOwner",
            },
          },
        ],
      },
    });

    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() => {
      root?.render(
        <ThemeProvider>
          <I18nProvider>
            <SystemNotificationProvider>
              <MemoryRouter>
                <SharedUpstreamAccountDetailDrawer
                  open
                  accountId={5}
                  initialTab="healthEvents"
                  onClose={vi.fn()}
                />
              </MemoryRouter>
            </SystemNotificationProvider>
          </I18nProvider>
        </ThemeProvider>,
      );
    });

    await flushAsync();
    expect(document.body.textContent).toContain("加密 owner 约束");

    const openButton = document.body.querySelector(
      '[data-testid="upstream-account-recent-action-open-blocked-binding"]',
    );
    if (!(openButton instanceof HTMLButtonElement)) {
      throw new Error("missing blocked binding action button");
    }

    act(() => {
      openButton.click();
    });

    expect(navigateMock).toHaveBeenCalledWith({
      pathname: "/dashboard",
      search:
        "?blockedBindingUpstreamAccountId=2890&blockedBindingConstraintSource=encryptedSessionOwner",
    });
  });

  it.skip("locates a health event invocation in the legacy records tab", async () => {
    mockAccountsPage();
    apiMocks.fetchInvocationRecordLocation.mockResolvedValue({
      anchorId: "anchor-test-001",
      snapshotId: 91,
      total: 121,
      page: 2,
      pageSize: 50,
      targetIndex: 0,
      targetAbsoluteIndex: 50,
      records: [
        {
          id: 51,
          invokeId: "invk_action_001",
          occurredAt: "2026-03-16T02:06:00.000Z",
          createdAt: "2026-03-16T02:06:00.000Z",
          status: "failed",
          model: "gpt-5.4",
          upstreamAccountId: 5,
          upstreamAccountName: "Existing OAuth",
          routeMode: "pool",
        },
      ],
    });

    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() => {
      root?.render(
        <ThemeProvider>
          <I18nProvider>
            <SystemNotificationProvider>
              <MemoryRouter>
                <SharedUpstreamAccountDetailDrawer
                  open
                  accountId={5}
                  initialTab="healthEvents"
                  onClose={vi.fn()}
                />
              </MemoryRouter>
            </SystemNotificationProvider>
          </I18nProvider>
        </ThemeProvider>,
      );
    });

    const invokeButton = Array.from(document.body.querySelectorAll("button")).find((button) =>
      button.textContent?.includes("invk_action_001"),
    );
    expect(invokeButton).toBeTruthy();
    act(() => {
      invokeButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushAsync();

    expect(apiMocks.fetchInvocationRecordLocation).toHaveBeenCalledWith({
      invokeId: "invk_action_001",
      upstreamAccountId: 5,
      pageSize: 50,
    });
    const recordsTab = Array.from(document.body.querySelectorAll('button[role="tab"]')).find(
      (button) => /请求|requests/i.test(button.textContent ?? ""),
    );
    expect(recordsTab?.getAttribute("aria-selected")).toBe("true");
    await waitForAssertion(() => {
      expect(document.body.querySelector('[data-testid="invocation-id"]')?.textContent).toBe(
        "invk_action_001",
      );
    });
    expect(virtualizerMocks.scrollToIndex).toHaveBeenCalledWith(0, {
      align: "center",
    });
    expect(apiMocks.fetchInvocationRecords).not.toHaveBeenCalled();
    expect(document.body.textContent).toMatch(/Return to latest requests|返回最新请求/);
  });

  it.skip("keeps the legacy records tab open and focuses a not-found locator alert", async () => {
    mockAccountsPage();
    apiMocks.fetchInvocationRecordLocation.mockRejectedValue(
      new ApiRequestError(404, "invocation_not_found"),
    );

    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() => {
      root?.render(
        <ThemeProvider>
          <I18nProvider>
            <SystemNotificationProvider>
              <MemoryRouter>
                <SharedUpstreamAccountDetailDrawer
                  open
                  accountId={5}
                  initialTab="healthEvents"
                  onClose={vi.fn()}
                />
              </MemoryRouter>
            </SystemNotificationProvider>
          </I18nProvider>
        </ThemeProvider>,
      );
    });

    const invokeButton = Array.from(document.body.querySelectorAll("button")).find((button) =>
      button.textContent?.includes("invk_action_001"),
    );
    act(() => {
      invokeButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flushAsync();

    await waitForAssertion(() => {
      expect(document.body.querySelector('[role="alert"]')?.textContent).toContain(
        "invk_action_001",
      );
    });
    const alert = document.body.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain("invk_action_001");
    await waitForAssertion(() => {
      expect(alert).toBe(document.activeElement);
    });
    const recordsTab = Array.from(document.body.querySelectorAll('button[role="tab"]')).find(
      (button) => /请求|requests/i.test(button.textContent ?? ""),
    );
    expect(recordsTab?.getAttribute("aria-selected")).toBe("true");
  });

  it.skip("clears a stale legacy records error after an SSE-open retry succeeds", async () => {
    mockAccountsPage();
    apiMocks.fetchInvocationRecords
      .mockRejectedValueOnce(new Error("initial records fetch failed"))
      .mockResolvedValueOnce({
        snapshotId: 84,
        total: 1,
        page: 1,
        pageSize: 50,
        records: [
          {
            id: 2,
            invokeId: "invoke-recovered",
            occurredAt: "2026-03-16T02:06:00.000Z",
            createdAt: "2026-03-16T02:06:00.000Z",
            status: "success",
            model: "gpt-5.4",
            upstreamAccountId: 5,
            upstreamAccountName: "Existing OAuth",
            routeMode: "pool",
          },
        ],
      });

    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() => {
      root?.render(
        <ThemeProvider>
          <I18nProvider>
            <SystemNotificationProvider>
              <MemoryRouter>
                <SharedUpstreamAccountDetailDrawer
                  open
                  accountId={5}
                  initialTab="overview"
                  onClose={vi.fn()}
                />
              </MemoryRouter>
            </SystemNotificationProvider>
          </I18nProvider>
        </ThemeProvider>,
      );
    });

    clickTab(/请求|requests/i);
    await flushAsync();
    await waitForAssertion(() => {
      expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledTimes(1);
    });
    await waitForAssertion(() => {
      expect(document.body.textContent).toContain("initial records fetch failed");
    });

    act(() => {
      sseMocks.onOpen?.();
    });
    await flushAsync();

    await waitForAssertion(() => {
      expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledTimes(2);
    });
    expect(document.body.textContent).not.toContain("initial records fetch failed");

    await waitForAssertion(() => {
      expect(renderedInvocationAccountNames()).toContain("Existing OAuth");
    });
    expect(document.body.textContent).not.toContain("initial records fetch failed");
  });

  it.skip("shows account activity and the legacy final-invocation infinite table", async () => {
    const secondFetch = deferred<{
      snapshotId: number;
      total: number;
      page: number;
      pageSize: number;
      records: Array<Record<string, unknown>>;
    }>();

    mockAccountsPage();
    apiMocks.fetchInvocationRecords
      .mockResolvedValueOnce({
        snapshotId: 42,
        total: 51,
        page: 1,
        pageSize: 50,
        records: [
          {
            id: 1,
            invokeId: "invoke-stable",
            occurredAt: "2026-03-16T02:05:00.000Z",
            createdAt: "2026-03-16T02:05:00.000Z",
            status: "success",
            model: "gpt-5.4",
            upstreamAccountId: 5,
            upstreamAccountName: "Existing OAuth",
            routeMode: "pool",
          },
        ],
      })
      .mockImplementationOnce(async () => secondFetch.promise as never);

    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() => {
      root?.render(
        <ThemeProvider>
          <I18nProvider>
            <SystemNotificationProvider>
              <MemoryRouter>
                <SharedUpstreamAccountDetailDrawer
                  open
                  accountId={5}
                  initialTab="overview"
                  onClose={vi.fn()}
                />
              </MemoryRouter>
            </SystemNotificationProvider>
          </I18nProvider>
        </ThemeProvider>,
      );
    });

    await waitForAssertion(() => {
      expect(
        document.body.querySelector('[data-testid="upstream-account-records-activity-overview"]'),
      ).toBeTruthy();
    });
    expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledTimes(0);

    clickTab(/请求|requests/i);
    await flushAsync();
    await waitForAssertion(() => {
      expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledTimes(1);
    });
    expect(
      document.body.querySelector('[data-testid="upstream-account-records-activity-overview"]'),
    ).toBeNull();
    expect(document.body.textContent).not.toMatch(/记录数量|Rows/);
    expect(document.body.textContent).toMatch(
      /最近 7 天主库中的尝试请求|request attempts from the primary database/i,
    );
    expect(document.body.textContent).toMatch(/请求 ID|request id/i);
    expect(renderedInvocationAccountNames()).toContain("Existing OAuth");

    const drawerBody = document.body.querySelector(".drawer-body");
    if (!(drawerBody instanceof HTMLElement)) {
      throw new Error("missing drawer body");
    }
    Object.defineProperty(drawerBody, "scrollHeight", {
      configurable: true,
      value: 1200,
    });
    Object.defineProperty(drawerBody, "clientHeight", {
      configurable: true,
      value: 800,
    });
    Object.defineProperty(drawerBody, "scrollTop", {
      configurable: true,
      value: 390,
    });
    act(() => {
      drawerBody.dispatchEvent(new Event("scroll"));
    });

    await waitForAssertion(() => {
      expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledTimes(2);
    });
    expect(apiMocks.fetchInvocationRecords.mock.calls[1]?.[0]).toMatchObject({
      upstreamAccountId: 5,
      page: 2,
      pageSize: 50,
      snapshotId: 42,
      sortBy: "occurredAt",
      sortOrder: "desc",
    });
    expect(renderedInvocationAccountNames()).toContain("Existing OAuth");
    expect(document.body.textContent).toMatch(/Loading more request attempts|正在加载更多尝试请求/);

    act(() => {
      secondFetch.resolve({
        snapshotId: 84,
        total: 2,
        page: 2,
        pageSize: 50,
        records: [
          {
            id: 2,
            invokeId: "invoke-next",
            occurredAt: "2026-03-16T02:04:00.000Z",
            createdAt: "2026-03-16T02:04:00.000Z",
            status: "success",
            model: "gpt-5.4",
            upstreamAccountId: 5,
            upstreamAccountName: "Existing OAuth",
            routeMode: "pool",
          },
        ],
      } as never);
    });
    await flushAsync();

    await waitForAssertion(() => {
      expect(renderedInvocationAccountNames()).toContain("Existing OAuth");
    });
    expect(renderedInvocationAccountNames()).toHaveLength(2);
    expect(document.body.textContent).toMatch(
      /All 2 request attempts loaded|已加载全部 2 条尝试请求/,
    );
  }, 30000);

  it.skip("clears stale legacy final-invocation rows when entering the records tab", async () => {
    const secondFetch = deferred<{
      snapshotId: number;
      total: number;
      page: number;
      pageSize: number;
      records: Array<Record<string, unknown>>;
    }>();

    mockAccountsPage();
    apiMocks.fetchInvocationRecords
      .mockResolvedValueOnce({
        snapshotId: 42,
        total: 1,
        page: 1,
        pageSize: 50,
        records: [
          {
            id: 1,
            invokeId: "invoke-old-tab",
            occurredAt: "2026-03-16T02:05:00.000Z",
            createdAt: "2026-03-16T02:05:00.000Z",
            status: "success",
            model: "gpt-5.4",
            upstreamAccountId: 5,
            upstreamAccountName: "Existing OAuth",
            routeMode: "pool",
          },
        ],
      })
      .mockImplementationOnce(async () => secondFetch.promise as never);

    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() => {
      root?.render(
        <ThemeProvider>
          <I18nProvider>
            <SystemNotificationProvider>
              <MemoryRouter>
                <SharedUpstreamAccountDetailDrawer
                  open
                  accountId={5}
                  initialTab="overview"
                  onClose={vi.fn()}
                />
              </MemoryRouter>
            </SystemNotificationProvider>
          </I18nProvider>
        </ThemeProvider>,
      );
    });

    await waitForAssertion(() => {
      expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledTimes(0);
    });
    clickTab(/请求|requests/i);
    await flushAsync();
    await waitForAssertion(() => {
      expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledTimes(1);
    });
    expect(renderedInvocationAccountNames()).toContain("Existing OAuth");

    clickTab(/概览|overview/i);
    await flushAsync();
    clickTab(/请求|requests/i);
    await flushAsync();

    await waitForAssertion(() => {
      expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledTimes(2);
    });
    expect(renderedInvocationAccountNames()).toHaveLength(0);
    expect(
      document.body.querySelector('[aria-label="正在加载记录"], [aria-label="Loading records"]'),
    ).toBeTruthy();

    act(() => {
      secondFetch.resolve({
        snapshotId: 84,
        total: 1,
        page: 1,
        pageSize: 50,
        records: [
          {
            id: 2,
            invokeId: "invoke-new-tab",
            occurredAt: "2026-03-16T02:06:00.000Z",
            createdAt: "2026-03-16T02:06:00.000Z",
            status: "success",
            model: "gpt-5.4",
            upstreamAccountId: 5,
            upstreamAccountName: "Existing OAuth",
            routeMode: "pool",
          },
        ],
      } as never);
    });
    await flushAsync();

    await waitForAssertion(() => {
      expect(renderedInvocationAccountNames()).toContain("Existing OAuth");
    });
  });
});
