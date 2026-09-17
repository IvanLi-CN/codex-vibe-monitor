/** @vitest-environment jsdom */
import userEvent from "@testing-library/user-event";
import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeAll, beforeEach, expect, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import type {
  PromptCacheConversation,
  PromptCacheConversationBindingResponse,
  PromptCacheConversationsResponse,
  UpdatePromptCacheConversationBindingPayload,
  UpstreamAccountDetail,
  UpstreamAccountSummary,
} from "../../lib/api";
import { PromptCacheConversationTable } from "./PromptCacheConversationTable";

type TestBindingResponse = Pick<
  PromptCacheConversationBindingResponse,
  "promptCacheKey" | "bindingKind"
> &
  Partial<Omit<PromptCacheConversationBindingResponse, "promptCacheKey" | "bindingKind">>;

const apiMocks = vi.hoisted(() => ({
  fetchUpstreamAccountDetail: vi.fn<(accountId: number) => Promise<UpstreamAccountDetail>>(),
  fetchInvocationRecords: vi.fn(),
  fetchInvocationRecordsSummary: vi.fn(),
  fetchPromptCacheConversationBinding:
    vi.fn<(promptCacheKey: string) => Promise<TestBindingResponse>>(),
  fetchPromptCacheConversationOperationEvents: vi.fn(),
  fetchUpstreamAccounts: vi.fn(),
  resetPromptCacheConversationAffinity:
    vi.fn<(promptCacheKey: string) => Promise<TestBindingResponse>>(),
  updatePromptCacheConversationBinding:
    vi.fn<
      (
        promptCacheKey: string,
        payload: UpdatePromptCacheConversationBindingPayload,
      ) => Promise<TestBindingResponse>
    >(),
}));
const detailTopicMocks = vi.hoisted(() => ({
  current: {
    calls: { data: null as unknown, lastKind: null as "snapshot" | "replay" | "live" | null },
    overview: { data: null as unknown, lastKind: null as "snapshot" | "replay" | "live" | null },
    binding: { data: null as unknown, lastKind: null as "snapshot" | "replay" | "live" | null },
    operations: {
      data: null as unknown,
      lastKind: null as "snapshot" | "replay" | "live" | null,
    },
    isSseUnavailable: false,
  },
}));
class MockPointerEvent extends MouseEvent {
  pointerType: string;
  constructor(type: string, init: MouseEventInit & { pointerType?: string } = {}) {
    super(type, init);
    this.pointerType = init.pointerType ?? "mouse";
  }
}
function findSelectOption(label: string) {
  return Array.from(document.querySelectorAll('[role="option"]')).find((option) =>
    option.textContent?.includes(label),
  ) as HTMLElement | undefined;
}
function requireTestValue<T>(value: T | null | undefined, label: string): T {
  if (value == null) {
    throw new Error(`missing test value: ${label}`);
  }
  return value;
}
vi.mock("../../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../../lib/api")>("../../lib/api");
  return {
    ...actual,
    fetchUpstreamAccountDetail: apiMocks.fetchUpstreamAccountDetail,
    fetchInvocationRecords: apiMocks.fetchInvocationRecords,
    fetchInvocationRecordsSummary: apiMocks.fetchInvocationRecordsSummary,
    fetchPromptCacheConversationBinding: apiMocks.fetchPromptCacheConversationBinding,
    fetchPromptCacheConversationOperationEvents:
      apiMocks.fetchPromptCacheConversationOperationEvents,
    fetchUpstreamAccounts: apiMocks.fetchUpstreamAccounts,
    resetPromptCacheConversationAffinity: apiMocks.resetPromptCacheConversationAffinity,
    updatePromptCacheConversationBinding: apiMocks.updatePromptCacheConversationBinding,
  };
});
vi.mock("../../lib/sse", async () => {
  const actual = await vi.importActual<typeof import("../../lib/sse")>("../../lib/sse");
  return {
    ...actual,
    subscribeToTopic: () => () => {},
  };
});
vi.mock("../../hooks/useConversationDetailTopics", () => ({
  resolveConversationDetailScope: (conversationKey: string | null) =>
    conversationKey ? { promptCacheKey: conversationKey } : null,
  useConversationDetailTopics: () => detailTopicMocks.current,
}));
function renderTable(stats: PromptCacheConversationsResponse) {
  return renderToStaticMarkup(
    <I18nProvider>
      <PromptCacheConversationTable stats={stats} isLoading={false} error={null} />
    </I18nProvider>,
  );
}
function formatZhDateTime(raw: string) {
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(new Date(raw));
}
function createConversation(
  overrides: Partial<PromptCacheConversation> & {
    promptCacheKey: string;
    createdAt: string;
    lastActivityAt: string;
  },
): PromptCacheConversation {
  return {
    promptCacheKey: overrides.promptCacheKey,
    requestCount: overrides.requestCount ?? 1,
    totalTokens: overrides.totalTokens ?? 0,
    totalCost: overrides.totalCost ?? 0,
    createdAt: overrides.createdAt,
    lastActivityAt: overrides.lastActivityAt,
    hasEncryptedSessionOwner: overrides.hasEncryptedSessionOwner ?? false,
    encryptedOwnerAccountId: overrides.encryptedOwnerAccountId ?? null,
    encryptedOwnerAccountName: overrides.encryptedOwnerAccountName ?? null,
    encryptedOwnerGroupName: overrides.encryptedOwnerGroupName ?? null,
    upstreamAccounts: overrides.upstreamAccounts ?? [],
    recentInvocations: overrides.recentInvocations ?? [],
    last24hRequests: overrides.last24hRequests ?? [],
  };
}
function createUpstreamAccountSummary(
  id: number,
  displayName: string,
  groupName: string,
  overrides: Partial<UpstreamAccountSummary> = {},
) {
  return {
    id,
    kind: "api_key_codex",
    provider: "codex",
    displayName,
    groupName,
    isMother: false,
    status: "active",
    workStatus: "idle",
    enableStatus: "enabled",
    healthStatus: "normal",
    syncState: "idle",
    displayStatus: "active",
    enabled: true,
    email: null,
    chatgptAccountId: null,
    planType: null,
    maskedApiKey: "sk-***",
    tags: [],
    effectiveRoutingRule: {
      allowCutOut: true,
      allowCutIn: true,
      sourceTagIds: [],
      sourceTagNames: [],
    },
    ...overrides,
  };
}
let host: HTMLDivElement | null = null;
let root: Root | null = null;
let consoleErrorSpy: ReturnType<typeof vi.spyOn> | null = null;
beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
  Object.defineProperty(window, "PointerEvent", {
    configurable: true,
    writable: true,
    value: MockPointerEvent,
  });
  Object.defineProperty(globalThis, "PointerEvent", {
    configurable: true,
    writable: true,
    value: MockPointerEvent,
  });
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    writable: true,
    value: () => undefined,
  });
  Object.defineProperty(HTMLElement.prototype, "hasPointerCapture", {
    configurable: true,
    writable: true,
    value: () => false,
  });
  Object.defineProperty(HTMLElement.prototype, "setPointerCapture", {
    configurable: true,
    writable: true,
    value: () => undefined,
  });
  Object.defineProperty(HTMLElement.prototype, "releasePointerCapture", {
    configurable: true,
    writable: true,
    value: () => undefined,
  });
});
function configurePromptCacheMocks() {
  apiMocks.fetchUpstreamAccountDetail.mockReset();
  apiMocks.fetchInvocationRecords.mockReset();
  apiMocks.fetchInvocationRecordsSummary.mockReset();
  apiMocks.fetchPromptCacheConversationBinding.mockReset();
  apiMocks.fetchPromptCacheConversationOperationEvents.mockReset();
  apiMocks.fetchUpstreamAccounts.mockReset();
  apiMocks.resetPromptCacheConversationAffinity.mockReset();
  apiMocks.updatePromptCacheConversationBinding.mockReset();
  detailTopicMocks.current = {
    calls: { data: null as unknown, lastKind: null },
    overview: { data: null as unknown, lastKind: null },
    binding: { data: null as unknown, lastKind: null },
    operations: { data: null as unknown, lastKind: null },
    isSseUnavailable: false,
  };
  apiMocks.fetchPromptCacheConversationBinding.mockResolvedValue({
    promptCacheKey: "pck-history",
    bindingKind: "none",
    groupName: null,
    upstreamAccountId: null,
    upstreamAccountName: null,
    policyFieldSources: {
      allowSwitchUpstream: "account",
      fastModeRewriteMode: "account",
      imageToolRewriteMode: "account",
      availableModels: "account",
      forwardProxyKey: "account",
    },
    updatedAt: null,
  });
  apiMocks.fetchUpstreamAccounts.mockResolvedValue({
    writesEnabled: true,
    items: [],
    groups: [],
    forwardProxyNodes: [],
    hasUngroupedAccounts: false,
    total: 0,
    page: 1,
    pageSize: 500,
    metrics: { total: 0, oauth: 0, apiKey: 0, attention: 0 },
    routing: null,
  });
  apiMocks.fetchPromptCacheConversationOperationEvents.mockResolvedValue({
    items: [],
    total: 0,
    page: 1,
    pageSize: 20,
  });
  apiMocks.updatePromptCacheConversationBinding.mockResolvedValue({
    promptCacheKey: "pck-history",
    bindingKind: "none",
    groupName: null,
    upstreamAccountId: null,
    upstreamAccountName: null,
    updatedAt: null,
  });
  apiMocks.resetPromptCacheConversationAffinity.mockResolvedValue({
    promptCacheKey: "pck-history",
    bindingKind: "none",
    groupName: null,
    upstreamAccountId: null,
    upstreamAccountName: null,
    updatedAt: null,
  });
  apiMocks.fetchInvocationRecordsSummary.mockResolvedValue({
    snapshotId: 900,
    newRecordsCount: 0,
    totalCount: 0,
    successCount: 0,
    failureCount: 0,
    totalCost: 0,
    totalTokens: 0,
    token: {
      requestCount: 0,
      totalTokens: 0,
      avgTokensPerRequest: 0,
      cacheInputTokens: 0,
      totalCost: 0,
    },
    network: {
      avgTtfbMs: null,
      p95TtfbMs: null,
      avgTotalMs: null,
      p95TotalMs: null,
    },
    exception: {
      failureCount: 0,
      serviceFailureCount: 0,
      clientFailureCount: 0,
      clientAbortCount: 0,
      actionableFailureCount: 0,
    },
  });
}
beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-03-03T00:00:00Z"));
  consoleErrorSpy = vi.spyOn(console, "error").mockImplementation((...args: unknown[]) => {
    const [firstArg] = args;
    const message = typeof firstArg === "string" ? firstArg : String(firstArg ?? "");
    if (
      message.includes("not wrapped in act") ||
      message.includes("Not implemented: Window's scrollTo() method")
    ) {
      return;
    }
  });
  configurePromptCacheMocks();
});
afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  consoleErrorSpy?.mockRestore();
  consoleErrorSpy = null;
  vi.useRealTimers();
});
function renderInteractiveElement(element: React.ReactNode) {
  if (!host) {
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  }
  act(() => {
    root?.render(
      <MemoryRouter>
        <I18nProvider>{element}</I18nProvider>
      </MemoryRouter>,
    );
  });
}
function renderInteractive(
  stats: PromptCacheConversationsResponse | null,
  props: Partial<ComponentProps<typeof PromptCacheConversationTable>> = {},
) {
  renderInteractiveElement(
    <PromptCacheConversationTable stats={stats} isLoading={false} error={null} {...props} />,
  );
}
function findButtonByAriaLabel(label: string, index = 0) {
  return (
    Array.from(document.querySelectorAll("button")).filter(
      (button): button is HTMLButtonElement =>
        button.getAttribute("aria-label") === label || button.textContent?.includes(label) === true,
    )[index] ?? null
  );
}
function findInputByAriaLabel(label: string) {
  return document.querySelector(`input[aria-label="${label}"]`) as HTMLInputElement | null;
}
async function clickDrawerTab(label: string) {
  if ((label === "设置" || label === "路由") && detailTopicMocks.current.binding.data == null) {
    detailTopicMocks.current.isSseUnavailable = true;
  }
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  const tab = Array.from(document.querySelectorAll('[role="tab"]')).find((node) =>
    node.textContent?.includes(label),
  ) as HTMLElement | undefined;
  expect(tab).toBeTruthy();
  await user.click(requireTestValue(tab, "drawer tab"));
  await flushInteractive();
}
async function flushInteractive() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

export {
  apiMocks,
  clickDrawerTab,
  consoleErrorSpy,
  createConversation,
  createUpstreamAccountSummary,
  detailTopicMocks,
  findButtonByAriaLabel,
  findInputByAriaLabel,
  findSelectOption,
  flushInteractive,
  formatZhDateTime,
  host,
  MockPointerEvent,
  renderInteractive,
  renderInteractiveElement,
  renderTable,
  requireTestValue,
  root,
};
