/* eslint-disable @typescript-eslint/ban-ts-comment, @typescript-eslint/no-unused-vars */
// @ts-nocheck
/** @vitest-environment jsdom */
import * as React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { createMemoryRouter, MemoryRouter, Route, RouterProvider, Routes } from "react-router-dom";
import ts from "typescript";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { SystemNotificationProvider } from "../../components/ui/system-notifications";
import { I18nProvider } from "../../i18n";
import { ApiRequestError } from "../../lib/api";
import { ThemeProvider } from "../../theme/context";
import UpstreamAccountsPage, { SharedUpstreamAccountDetailDrawer } from "./UpstreamAccounts";
import suite5 from "./UpstreamAccounts.api-key-details.txt?raw";
import suite6 from "./UpstreamAccounts.delete-confirmation.txt?raw";
import suite2 from "./UpstreamAccounts.duplicates.txt?raw";
import suite7 from "./UpstreamAccounts.edit-drafts.txt?raw";
import suite8 from "./UpstreamAccounts.legacy-records-skipped.txt?raw";
import suite9 from "./UpstreamAccounts.mock-accounts-page.txt?raw";
import suite4 from "./UpstreamAccounts.oauth-recovery.txt?raw";
import suite1 from "./UpstreamAccounts.roster-freshness.txt?raw";
import suite3 from "./UpstreamAccounts.sync-state-isolation.txt?raw";
import {
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
  renderedInvocationAccountNames,
  renderFlatForLegacySuites,
  rerender,
  root,
  setComboboxValue,
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
} from "./UpstreamAccounts.test-support-base";

const rosterFreshnessItems = [
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
] as const;
const defaultRosterFreshnessGroups = [
  {
    groupName: "prod",
    accountCount: 2,
    note: "prod note",
    boundProxyKeys: ["jp-edge-01"],
    nodeShuntEnabled: false,
    upstream429RetryEnabled: false,
    upstream429MaxRetries: 0,
  },
];

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
    items: rosterFreshnessItems,
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
    groups: options?.groups ?? defaultRosterFreshnessGroups,
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
  ApiRequestError,
  ThemeProvider,
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
  renderedInvocationAccountNames,
  renderFlatForLegacySuites,
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
  expandLoginHealthDetails,
  clickDrawerBackdrop,
  clickDrawerGutter,
  clickFirstRosterRow,
  clickCheckboxByLabel,
  clickCombobox,
  clickCommandItem,
  sseMocks,
  virtualizerMocks,
  waitForAssertion,
  pressButton,
  defaultEffectiveRoutingRule,
  defaultPoolTags,
  deferred,
  buildBulkSyncCounts,
  buildBulkSyncSnapshot,
  buildBulkSyncSnapshotEvent,
  buildBulkSyncJobResponse,
  mockBulkSyncPage,
  mockRosterFreshnessPage,
});
Object.defineProperties(scope, {
  host: {
    get: () => host,
    set: (value) => {
      setHost(value as typeof host);
    },
  },
  root: {
    get: () => root,
    set: (value) => {
      setRoot(value as typeof root);
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
evalChunk(suite9);
const mockAccountsPage = scope.mockAccountsPage as (
  options?: Record<string, unknown>,
) => Record<string, unknown>;
evalChunk(suite1);
evalChunk(suite2);
evalChunk(suite3);
evalChunk(suite4);
evalChunk(suite5);
evalChunk(suite6);
evalChunk(suite7);
evalChunk(suite8);

export { mockAccountsPage };
