import { type ReactNode, useEffect, useRef } from "react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import type {
  ApiPoolUpstreamRequestAttempt,
  CompleteOauthLoginSessionPayload,
  CreateApiKeyAccountPayload,
  EffectiveRoutingRule,
  ImportOauthCredentialFilePayload,
  OauthMailboxStatus,
  StatsResponse,
  TimeseriesResponse,
  UpdateGroupAccountRoutingRulePayload,
  UpdateOauthLoginSessionPayload,
  UpdatePoolRoutingSettingsPayload,
  UpdateUpstreamAccountGroupPayload,
  UpdateUpstreamAccountPayload,
  UpstreamAccountListResponse,
} from "../../lib/api";
import AccountPoolLayout from "../../pages/account-pool/AccountPoolLayout";
import GroupsPage from "../../pages/account-pool/Groups";
import MaintenanceRecordsPage from "../../pages/account-pool/MaintenanceRecords";
import UpstreamAccountCreatePage from "../../pages/account-pool/UpstreamAccountCreate";
import { resolveDisplayNameAfterEmailChange } from "../../pages/account-pool/UpstreamAccountCreate.shared";
import UpstreamAccountsPage from "../../pages/account-pool/UpstreamAccounts";
import { useTheme } from "../../theme/context";

function applyRoutingRulePatchToEffectiveRule(
  rule: EffectiveRoutingRule,
  patch: UpdateGroupAccountRoutingRulePayload,
): EffectiveRoutingRule {
  const fieldSources = {
    allowCutOut: rule.fieldSources?.allowCutOut ?? "root",
    allowCutIn: rule.fieldSources?.allowCutIn ?? "root",
    priorityTier: rule.fieldSources?.priorityTier ?? "root",
    fastModeRewriteMode: rule.fieldSources?.fastModeRewriteMode ?? "root",
    imageToolRewriteMode: rule.fieldSources?.imageToolRewriteMode ?? "root",
    requestCompressionAlgorithm: rule.fieldSources?.requestCompressionAlgorithm ?? "root",
    concurrencyLimit: rule.fieldSources?.concurrencyLimit ?? "root",
    upstream429Retry: rule.fieldSources?.upstream429Retry ?? "root",
    availableModels: rule.fieldSources?.availableModels ?? "root",
    systemDeniedModels: rule.fieldSources?.systemDeniedModels ?? "root",
  };
  return {
    ...rule,
    ...(patch.allowCutOut == null ? {} : { allowCutOut: patch.allowCutOut }),
    ...(patch.allowCutIn == null ? {} : { allowCutIn: patch.allowCutIn }),
    ...(patch.priorityTier == null ? {} : { priorityTier: patch.priorityTier }),
    ...(patch.fastModeRewriteMode == null
      ? {}
      : { fastModeRewriteMode: patch.fastModeRewriteMode }),
    ...(patch.imageToolRewriteMode == null
      ? {}
      : { imageToolRewriteMode: patch.imageToolRewriteMode }),
    ...(patch.requestCompressionAlgorithm == null
      ? {}
      : { requestCompressionAlgorithm: patch.requestCompressionAlgorithm }),
    ...(patch.concurrencyLimit == null ? {} : { concurrencyLimit: patch.concurrencyLimit }),
    ...(patch.upstream429RetryEnabled == null
      ? {}
      : { upstream429RetryEnabled: patch.upstream429RetryEnabled }),
    ...(patch.upstream429MaxRetries == null
      ? {}
      : { upstream429MaxRetries: patch.upstream429MaxRetries }),
    ...(patch.availableModels == null ? {} : { availableModels: patch.availableModels }),
    fieldSources: {
      ...fieldSources,
      ...(Object.hasOwn(patch, "allowCutOut")
        ? { allowCutOut: patch.allowCutOut == null ? "root" : "account" }
        : {}),
      ...(Object.hasOwn(patch, "allowCutIn")
        ? { allowCutIn: patch.allowCutIn == null ? "root" : "account" }
        : {}),
      ...(Object.hasOwn(patch, "priorityTier")
        ? { priorityTier: patch.priorityTier == null ? "root" : "account" }
        : {}),
      ...(Object.hasOwn(patch, "fastModeRewriteMode")
        ? { fastModeRewriteMode: patch.fastModeRewriteMode == null ? "root" : "account" }
        : {}),
      ...(Object.hasOwn(patch, "imageToolRewriteMode")
        ? { imageToolRewriteMode: patch.imageToolRewriteMode == null ? "root" : "account" }
        : {}),
      ...(Object.hasOwn(patch, "requestCompressionAlgorithm")
        ? {
            requestCompressionAlgorithm:
              patch.requestCompressionAlgorithm == null ? "root" : "account",
          }
        : {}),
      ...(Object.hasOwn(patch, "concurrencyLimit")
        ? { concurrencyLimit: patch.concurrencyLimit == null ? "root" : "account" }
        : {}),
      ...(Object.hasOwn(patch, "upstream429RetryEnabled")
        ? { upstream429Retry: patch.upstream429RetryEnabled == null ? "root" : "account" }
        : {}),
      ...(Object.hasOwn(patch, "availableModels")
        ? { availableModels: patch.availableModels == null ? "root" : "account" }
        : {}),
    },
  };
}

import {
  applyDynamicRosterLiveRefresh,
  buildBulkSyncCounts,
  buildBulkSyncSnapshot,
  buildBulkSyncSnapshotEvent,
  clone,
  createApiKeyAccount,
  createBulkSyncRows,
  createOauthAccount,
  createStore,
  currentStoryId,
  defaultRoutingMaintenance,
  defaultRoutingTimeouts,
  filterAccountsForQuery,
  isDynamicRosterStoryId,
  listGroupSummaries,
  listTagSummaries,
  maskApiKey,
  normalizeGroupName,
  now,
  type StoryBulkSyncJob,
  type StoryInitialEntry,
  type StoryStore,
  storyFutureExpiresAt,
  storyFutureLoginExpiresAt,
  storyHasExistingMailboxAddress,
  storyHealthStatus,
  storySyncState,
  storyWorkStatus,
  syncLocalWindows,
  toSummary,
  updateStoryBulkSyncJob,
} from "./UpstreamAccountsPage.story-runtime-core";
import {
  buildStoryImportedOauthValidationResponse,
  getAccountActivityResponseDelay,
  getRosterResponseDelay,
  getRosterResponseFailure,
  getWindowUsageResponseDelay,
  jsonResponse,
  MockStoryBulkSyncEventSource,
  noContent,
  parseBody,
  shouldKeepAccountActivityPending,
  wait,
} from "./UpstreamAccountsPage.story-runtime-fetch-helpers";
import {
  buildStickyConversations,
  buildStickyInvocationRecords,
} from "./UpstreamAccountsPage.story-runtime-sticky";

export type { StoryInitialEntry } from "./UpstreamAccountsPage.story-runtime-core";

declare global {
  interface Window {
    __storybookUpstreamAccountsController__?: {
      getRequestLog: () => string[];
      clearRequestLog: () => void;
    };
  }
}

function stripActualUsageFromRosterWindow<T extends { actualUsage?: unknown } | null | undefined>(
  window: T,
): T {
  if (!window || typeof window !== "object") return window;
  return {
    ...window,
    actualUsage: null,
  } as T;
}

function buildStoryWindowActualUsage(accountId: number, multiplier: number) {
  const requestCount = Math.max(1, Math.round((accountId % 17) + multiplier * 3));
  const totalTokens = requestCount * 3200 + accountId * 11;
  const totalCost = Number((requestCount * 0.041 + multiplier * 0.09).toFixed(4));
  const cacheInputTokens = Math.round(totalTokens * 0.12);
  const inputTokens = Math.round(totalTokens * 0.56);
  const outputTokens = totalTokens - inputTokens - cacheInputTokens;
  return {
    requestCount,
    totalTokens,
    totalCost,
    inputTokens,
    outputTokens,
    cacheInputTokens,
  };
}

function storyAccountActivityIsEmpty(storyId: string | null) {
  return storyId?.endsWith("--detail-drawer-records-empty") === true;
}

function storyAccountActivityUsesOverflowFixture(storyId: string | null) {
  return storyId?.endsWith("--detail-drawer-records-overflow-dark-narrow") === true;
}

function buildStoryAccountActivitySummary(
  accountId: number,
  storyId: string | null,
): StatsResponse {
  if (storyAccountActivityIsEmpty(storyId)) {
    return {
      totalCount: 0,
      successCount: 0,
      failureCount: 0,
      totalCost: 0,
      totalTokens: 0,
    };
  }
  if (storyAccountActivityUsesOverflowFixture(storyId)) {
    return {
      totalCount: 2210,
      successCount: 1836,
      failureCount: 374,
      totalCost: 173.3,
      totalTokens: 281_110_000,
    };
  }
  const scale = accountId === 101 ? 1 : 0.35;
  const totalCount = Math.max(1, Math.round(37 * scale));
  const failureCount = accountId === 101 ? 3 : 1;
  return {
    totalCount,
    successCount: Math.max(0, totalCount - failureCount),
    failureCount,
    totalCost: Number((1.846 * scale).toFixed(4)),
    totalTokens: Math.round(1_284_600 * scale),
  };
}

function buildStoryAccountActivityTimeseries(
  accountId: number,
  parsedUrl: URL,
  storyId: string | null,
): TimeseriesResponse {
  const range = parsedUrl.searchParams.get("range") || "today";
  const bucket = parsedUrl.searchParams.get("bucket") || "1m";
  const bucketSeconds = bucket === "1h" ? 3_600 : bucket === "1d" ? 86_400 : 60;
  const rangeStart = "2026-03-13T00:00:00.000Z";
  if (storyAccountActivityUsesOverflowFixture(storyId) && range === "today") {
    const overflowPoints = Array.from({ length: 12 }, (_, index) => {
      const bucketStart = new Date(Date.parse(rangeStart) + index * bucketSeconds * 1_000);
      const totalCount = [3, 6, 12, 4, 0, 8, 13, 7, 2, 11, 5, 9][index] ?? 0;
      const failureCount = [0, 1, 3, 0, 0, 1, 4, 2, 0, 3, 1, 2][index] ?? 0;
      const successCount = Math.max(totalCount - failureCount, 0);
      const totalTokens =
        [
          810_000, 1_640_000, 2_940_000, 1_120_000, 0, 1_980_000, 3_220_000, 1_760_000, 620_000,
          2_880_000, 1_410_000, 2_340_000,
        ][index] ?? 0;
      const totalCost =
        [0.48, 0.96, 1.72, 0.61, 0, 1.11, 1.94, 1.03, 0.34, 1.76, 0.82, 1.39][index] ?? 0;
      return {
        bucketStart: bucketStart.toISOString(),
        bucketEnd: new Date(bucketStart.getTime() + bucketSeconds * 1_000).toISOString(),
        totalCount,
        successCount,
        failureCount,
        inFlightCount: 0,
        totalTokens,
        totalCost,
      };
    });
    return {
      rangeStart,
      rangeEnd: new Date(
        Date.parse(rangeStart) + overflowPoints.length * bucketSeconds * 1_000,
      ).toISOString(),
      bucketSeconds,
      snapshotId: 1,
      effectiveBucket: bucket,
      availableBuckets: ["1m", "10m", "1h", "1d"],
      bucketLimitedToDaily: false,
      points: overflowPoints,
    };
  }
  const points = storyAccountActivityIsEmpty(storyId)
    ? []
    : Array.from({ length: range === "7d" ? 14 : 12 }, (_, index) => {
        const bucketStart = new Date(Date.parse(rangeStart) + index * bucketSeconds * 1_000);
        const count = accountId === 101 ? (index % 5) + 1 : index % 3;
        return {
          bucketStart: bucketStart.toISOString(),
          bucketEnd: new Date(bucketStart.getTime() + bucketSeconds * 1_000).toISOString(),
          totalCount: count,
          successCount: Math.max(0, count - (index % 7 === 0 ? 1 : 0)),
          failureCount: index % 7 === 0 ? 1 : 0,
          inFlightCount: index % 9 === 0 ? 1 : 0,
          totalTokens: count * 12_800,
          totalCost: Number((count * 0.034).toFixed(4)),
        };
      });
  return {
    rangeStart,
    rangeEnd: new Date(
      Date.parse(rangeStart) + Math.max(1, points.length) * bucketSeconds * 1_000,
    ).toISOString(),
    bucketSeconds,
    snapshotId: 1,
    effectiveBucket: bucket,
    availableBuckets: ["1m", "10m", "1h", "1d"],
    bucketLimitedToDaily: false,
    points,
  };
}

export function StorybookUpstreamAccountsMock({ children }: { children: ReactNode }) {
  const storeRef = useRef<StoryStore>(createStore());
  const originalFetchRef = useRef<typeof window.fetch | null>(null);
  const originalEventSourceRef = useRef<typeof window.EventSource | null>(null);
  const installedRef = useRef(false);

  if (typeof window !== "undefined" && !installedRef.current) {
    installedRef.current = true;
    originalFetchRef.current = window.fetch.bind(window);
    originalEventSourceRef.current = window.EventSource;
    window.__storybookUpstreamAccountsController__ = {
      getRequestLog: () => [...storeRef.current.requestLog],
      clearRequestLog: () => {
        storeRef.current.requestLog = [];
      },
    };

    const mockedFetch: typeof window.fetch = async (input, init) => {
      const method = (
        init?.method || (input instanceof Request ? input.method : "GET")
      ).toUpperCase();
      const inputUrl =
        typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
      const parsedUrl = new URL(inputUrl, window.location.origin);
      const path = parsedUrl.pathname;
      const storyId = currentStoryId();
      const store = storeRef.current;
      store.requestLog.push(`${method} ${path}${parsedUrl.search}`);

      if (path === "/api/stats/summary" && method === "GET") {
        const accountId = Number(parsedUrl.searchParams.get("upstreamAccountId") || 0);
        if (shouldKeepAccountActivityPending(storyId)) {
          return new Promise<Response>(() => {});
        }
        const delayMs = getAccountActivityResponseDelay(storyId);
        if (delayMs > 0) {
          await wait(delayMs);
        }
        return jsonResponse(buildStoryAccountActivitySummary(accountId, storyId));
      }

      if (path === "/api/stats/timeseries" && method === "GET") {
        const accountId = Number(parsedUrl.searchParams.get("upstreamAccountId") || 0);
        if (shouldKeepAccountActivityPending(storyId)) {
          return new Promise<Response>(() => {});
        }
        const delayMs = getAccountActivityResponseDelay(storyId);
        if (delayMs > 0) {
          await wait(delayMs);
        }
        return jsonResponse(buildStoryAccountActivityTimeseries(accountId, parsedUrl, storyId));
      }

      if (path === "/api/settings" && method === "GET") {
        return jsonResponse({
          proxy: {
            hijackEnabled: true,
            mergeUpstreamEnabled: true,
            fastModeRewriteMode: "disabled",
            upstream429MaxRetries: 3,
            websocketEnabled: true,
            upstreamWebsocketDefaultEnabled: true,
            requestBodyLoggingEnabled: true,
            responseBodyLoggingEnabled: true,
            encryptedSessionOwnerRoutingEnabled: true,
            defaultHijackEnabled: true,
            models: [
              "gpt-5.6-sol",
              "gpt-5.6-terra",
              "gpt-5.6-luna",
              "gpt-5.5",
              "gpt-5.5-pro",
              "gpt-5.4",
              "gpt-5.4-pro",
              "gpt-5.3-codex",
              "gpt-5.2",
              "gpt-5.2-codex",
              "gpt-5.1-codex-max",
              "gpt-5.1-codex-mini",
            ],
            enabledModels: [
              "gpt-5.6-sol",
              "gpt-5.6-terra",
              "gpt-5.6-luna",
              "gpt-5.5",
              "gpt-5.5-pro",
              "gpt-5.4",
              "gpt-5.4-pro",
              "gpt-5.3-codex",
              "gpt-5.2",
              "gpt-5.2-codex",
              "gpt-5.1-codex-max",
              "gpt-5.1-codex-mini",
            ],
          },
          forwardProxy: {
            enabled: false,
            proxies: [],
            subscriptions: [],
            defaultProxyKey: null,
          },
          pricing: {
            catalogVersion: "storybook-routing-models",
            entries: [
              {
                model: "gpt-5.6-sol",
                inputPer1m: 5,
                outputPer1m: 30,
                cacheInputPer1m: 1,
                cacheReadPer1m: 1,
                cacheWritePer1m: 6.25,
                reasoningPer1m: 0,
                source: "storybook",
              },
              {
                model: "gpt-5.4-mini",
                inputPer1m: 1,
                outputPer1m: 4,
                cacheInputPer1m: 0.2,
                cacheReadPer1m: 0.2,
                cacheWritePer1m: null,
                reasoningPer1m: 0,
                source: "storybook",
              },
            ],
          },
        });
      }

      if (path === "/api/pool/upstream-accounts" && method === "GET") {
        if (isDynamicRosterStoryId(storyId)) {
          store.rosterFetchCount += 1;
          if (store.rosterFetchCount > 1) {
            applyDynamicRosterLiveRefresh(store, store.rosterFetchCount);
          }
        }
        const filteredItems = filterAccountsForQuery(store, parsedUrl);
        const rawPageSize = Number(parsedUrl.searchParams.get("pageSize") || 20);
        const requestedPageSize =
          Number.isFinite(rawPageSize) && rawPageSize > 0 ? rawPageSize : 20;
        const includeAll = parsedUrl.searchParams.get("includeAll") === "true";
        const total = filteredItems.length;
        const pageCount = Math.max(1, Math.ceil(total / requestedPageSize));
        const rawPage = Number(parsedUrl.searchParams.get("page") || 1);
        const requestedPage = Number.isFinite(rawPage) && rawPage > 0 ? rawPage : 1;
        const page = includeAll ? 1 : Math.min(requestedPage, pageCount);
        const start = (page - 1) * requestedPageSize;
        const pageItems = (
          includeAll ? filteredItems : filteredItems.slice(start, start + requestedPageSize)
        ).map((item) => {
          const rosterItem = clone(item);
          rosterItem.primaryWindow = stripActualUsageFromRosterWindow(rosterItem.primaryWindow);
          rosterItem.secondaryWindow = stripActualUsageFromRosterWindow(rosterItem.secondaryWindow);
          return rosterItem;
        });
        const payload: UpstreamAccountListResponse = {
          writesEnabled: store.writesEnabled,
          groups: listGroupSummaries(store),
          forwardProxyNodes: clone(store.forwardProxyNodes),
          hasUngroupedAccounts: store.accounts.some(
            (account) => !normalizeGroupName(account.groupName),
          ),
          routing: clone(store.routing),
          items: pageItems,
          total,
          page,
          pageSize: includeAll ? Math.max(total, requestedPageSize) : requestedPageSize,
          metrics: {
            total,
            oauth: filteredItems.filter((item) => item.kind === "oauth_codex").length,
            apiKey: filteredItems.filter((item) => item.kind === "api_key_codex").length,
            attention: filteredItems.filter((item) => {
              const derivedHealthStatus = storyHealthStatus(item);
              const derivedSyncState = storySyncState(item);
              return (
                derivedHealthStatus !== "normal" ||
                storyWorkStatus(item, derivedHealthStatus, derivedSyncState) === "rate_limited"
              );
            }).length,
          },
        };
        const delayMs = getRosterResponseDelay(storyId, parsedUrl);
        const failureMessage = getRosterResponseFailure(storyId, parsedUrl);
        if (delayMs > 0) {
          await wait(delayMs);
        }
        if (failureMessage) {
          return jsonResponse({ message: failureMessage }, 503);
        }
        return jsonResponse(payload);
      }

      if (path === "/api/pool/forward-proxy-binding-nodes" && method === "GET") {
        const requestedKeys = new Set(parsedUrl.searchParams.getAll("key"));
        const nodes = store.forwardProxyNodes.filter((node) => {
          if (requestedKeys.size === 0) return true;
          return (
            requestedKeys.has(node.key) ||
            (node.aliasKeys ?? []).some((key) => requestedKeys.has(key))
          );
        });
        return jsonResponse(clone(nodes));
      }

      if (path === "/api/pool/upstream-account-events" && method === "GET") {
        const accountFilter = parsedUrl.searchParams.get("account")?.trim().toLowerCase() || "";
        const groupFilter = parsedUrl.searchParams.get("group")?.trim().toLowerCase() || "";
        const proxyKeyFilter = parsedUrl.searchParams.get("proxyKey")?.trim().toLowerCase() || "";
        const resultFilter = parsedUrl.searchParams.get("result")?.trim().toLowerCase() || "";
        const rawPageSize = Number(parsedUrl.searchParams.get("pageSize") || 20);
        const requestedPageSize =
          Number.isFinite(rawPageSize) && rawPageSize > 0 ? rawPageSize : 20;
        const rawPage = Number(parsedUrl.searchParams.get("page") || 1);
        const requestedPage = Number.isFinite(rawPage) && rawPage > 0 ? rawPage : 1;
        const filteredEvents = store.maintenanceEvents.filter((event) => {
          const accountText =
            `${event.accountDisplayName ?? ""} ${event.accountGroupName ?? ""}`.toLowerCase();
          const proxyText =
            `${event.forwardProxyKey ?? ""} ${event.forwardProxyDisplayName ?? ""}`.toLowerCase();
          if (accountFilter && !accountText.includes(accountFilter)) return false;
          if (groupFilter && !(event.accountGroupName ?? "").toLowerCase().includes(groupFilter))
            return false;
          if (proxyKeyFilter && !proxyText.includes(proxyKeyFilter)) return false;
          if (resultFilter && (event.result ?? "").toLowerCase() !== resultFilter) return false;
          return true;
        });
        const total = filteredEvents.length;
        const pageCount = Math.max(1, Math.ceil(total / requestedPageSize));
        const page = Math.min(requestedPage, pageCount);
        const start = (page - 1) * requestedPageSize;
        return jsonResponse({
          items: filteredEvents.slice(start, start + requestedPageSize),
          total,
          page,
          pageSize: requestedPageSize,
        });
      }

      if (path === "/api/pool/upstream-accounts/window-usage" && method === "POST") {
        const body = parseBody<{ accountIds?: number[] }>(init?.body, {});
        const accountIds = Array.isArray(body.accountIds)
          ? body.accountIds.filter((accountId) => Number.isFinite(accountId) && accountId > 0)
          : [];
        const delayMs = getWindowUsageResponseDelay(storyId);
        if (delayMs > 0) {
          await wait(delayMs);
        }
        return jsonResponse({
          items: accountIds.map((accountId) => {
            const account = store.accounts.find((item) => item.id === accountId);
            return {
              accountId,
              primaryActualUsage: account?.primaryWindow
                ? buildStoryWindowActualUsage(accountId, 1)
                : null,
              secondaryActualUsage: account?.secondaryWindow
                ? buildStoryWindowActualUsage(accountId, 2)
                : null,
            };
          }),
        });
      }

      if (path === "/api/pool/tags" && method === "GET") {
        return jsonResponse({
          writesEnabled: store.writesEnabled,
          items: listTagSummaries(store),
        });
      }

      if (path === "/api/pool/upstream-accounts/oauth/imports/validate" && method === "POST") {
        const body = parseBody<{ items?: ImportOauthCredentialFilePayload[] }>(init?.body, {});
        const items = Array.isArray(body.items) ? body.items : [];
        return jsonResponse(buildStoryImportedOauthValidationResponse(items));
      }

      if (path === "/api/pool/upstream-accounts/bulk-sync-jobs" && method === "POST") {
        const body = parseBody<{ accountIds?: number[] }>(init?.body, {});
        const accountIds = Array.isArray(body.accountIds) ? body.accountIds : [];
        const rows = createBulkSyncRows(store, accountIds);
        const jobId = `story-bulk-sync-${Date.now()}`;
        const job: StoryBulkSyncJob = {
          jobId,
          snapshot: buildBulkSyncSnapshot(jobId, rows),
          counts: buildBulkSyncCounts(rows),
          error: null,
        };
        store.bulkSyncJobs[jobId] = job;
        return jsonResponse(
          {
            jobId,
            ...buildBulkSyncSnapshotEvent(job),
          },
          201,
        );
      }

      if (path.startsWith("/api/pool/upstream-accounts/bulk-sync-jobs/") && method === "GET") {
        const match = path.match(/^\/api\/pool\/upstream-accounts\/bulk-sync-jobs\/([^/]+)$/);
        if (!match) {
          return jsonResponse({ message: "not found" }, 404);
        }
        const jobId = decodeURIComponent(match[1]);
        const job = store.bulkSyncJobs[jobId];
        if (!job) {
          return jsonResponse({ message: "not found" }, 404);
        }
        return jsonResponse({
          jobId,
          ...buildBulkSyncSnapshotEvent(job),
        });
      }

      if (path.startsWith("/api/pool/upstream-accounts/bulk-sync-jobs/") && method === "DELETE") {
        const match = path.match(/^\/api\/pool\/upstream-accounts\/bulk-sync-jobs\/([^/]+)$/);
        if (match) {
          const jobId = decodeURIComponent(match[1]);
          const job = store.bulkSyncJobs[jobId];
          if (job) {
            updateStoryBulkSyncJob(job, job.snapshot.rows, "cancelled");
          }
        }
        return noContent();
      }

      if (path === "/api/pool/routing-settings" && method === "PUT") {
        const body = parseBody<UpdatePoolRoutingSettingsPayload>(init?.body, {});
        const trimmed = body.apiKey?.trim();
        store.routing = {
          ...store.routing,
          ...(trimmed
            ? {
                apiKeyConfigured: true,
                maskedApiKey: maskApiKey(trimmed),
              }
            : {}),
          ...(body.requestCompressionAlgorithm
            ? { requestCompressionAlgorithm: body.requestCompressionAlgorithm }
            : {}),
          ...(body.requestCompressionLevelPreset
            ? { requestCompressionLevelPreset: body.requestCompressionLevelPreset }
            : {}),
          ...(body.maintenance
            ? {
                maintenance: {
                  primarySyncIntervalSecs:
                    body.maintenance.primarySyncIntervalSecs ??
                    store.routing.maintenance?.primarySyncIntervalSecs ??
                    defaultRoutingMaintenance.primarySyncIntervalSecs,
                  secondarySyncIntervalSecs:
                    body.maintenance.secondarySyncIntervalSecs ??
                    store.routing.maintenance?.secondarySyncIntervalSecs ??
                    defaultRoutingMaintenance.secondarySyncIntervalSecs,
                  priorityAvailableAccountCap:
                    body.maintenance.priorityAvailableAccountCap ??
                    store.routing.maintenance?.priorityAvailableAccountCap ??
                    defaultRoutingMaintenance.priorityAvailableAccountCap,
                },
              }
            : {}),
          ...(body.timeouts
            ? {
                timeouts: {
                  responsesFirstByteTimeoutSecs:
                    body.timeouts.responsesFirstByteTimeoutSecs ??
                    store.routing.timeouts?.responsesFirstByteTimeoutSecs ??
                    defaultRoutingTimeouts.responsesFirstByteTimeoutSecs,
                  compactFirstByteTimeoutSecs:
                    body.timeouts.compactFirstByteTimeoutSecs ??
                    store.routing.timeouts?.compactFirstByteTimeoutSecs ??
                    defaultRoutingTimeouts.compactFirstByteTimeoutSecs,
                  imageFirstByteTimeoutSecs:
                    body.timeouts.imageFirstByteTimeoutSecs ??
                    store.routing.timeouts?.imageFirstByteTimeoutSecs ??
                    defaultRoutingTimeouts.imageFirstByteTimeoutSecs,
                  responsesStreamTimeoutSecs:
                    body.timeouts.responsesStreamTimeoutSecs ??
                    store.routing.timeouts?.responsesStreamTimeoutSecs ??
                    defaultRoutingTimeouts.responsesStreamTimeoutSecs,
                  compactStreamTimeoutSecs:
                    body.timeouts.compactStreamTimeoutSecs ??
                    store.routing.timeouts?.compactStreamTimeoutSecs ??
                    defaultRoutingTimeouts.compactStreamTimeoutSecs,
                },
              }
            : {}),
        };
        return jsonResponse(clone(store.routing));
      }

      if (path === "/api/pool/upstream-accounts/oauth/login-sessions" && method === "POST") {
        const body = parseBody<{
          displayName?: string;
          email?: string;
          groupName?: string;
          note?: string;
          groupNote?: string;
          tagIds?: number[];
          isMother?: boolean;
          mailboxSessionId?: string;
          mailboxAddress?: string;
        }>(init?.body, {});
        const loginId = `login_${Date.now()}`;
        const redirectUri = `http://localhost:431${String(store.nextId).slice(-1)}/oauth/callback`;
        const state = `state_${loginId}`;
        const session: StoryStore["sessions"][string] = {
          loginId,
          status: "pending",
          authUrl: `https://auth.openai.com/authorize?mock=1&loginId=${loginId}&state=${state}`,
          redirectUri,
          expiresAt: storyFutureLoginExpiresAt,
          accountId: null,
          error: null,
          displayName: body.displayName,
          email: body.email?.trim() || undefined,
          groupName: body.groupName,
          isMother: body.isMother,
          note: body.note,
          groupNote: body.groupNote,
          tagIds: Array.isArray(body.tagIds) ? body.tagIds : [],
          mailboxSessionId: body.mailboxSessionId,
          mailboxAddress: body.mailboxAddress,
          state,
        };
        store.sessions[loginId] = session;
        return jsonResponse(clone(session), 201);
      }

      if (path === "/api/pool/upstream-accounts/oauth/mailbox-sessions" && method === "POST") {
        const body = parseBody<{ emailAddress?: string }>(init?.body, {});
        const requestedAddress = body.emailAddress?.trim().toLowerCase() ?? "";
        const shouldDelayMailboxAttach =
          storyId === "account-pool-pages-upstream-account-create-oauth--mailbox-attach-flow" ||
          storyId === "account-pool-pages-upstream-account-create-oauth--mailbox-attach-pending" ||
          storyId ===
            "account-pool-pages-upstream-account-create-batch-oauth--mailbox-attach-flow" ||
          storyId ===
            "account-pool-pages-upstream-account-create-batch-oauth--mailbox-popover-edit" ||
          storyId ===
            "account-pool-pages-upstream-account-create-batch-oauth--mailbox-attach-pending";
        const shouldDelayMailboxGenerate =
          storyId === "account-pool-pages-upstream-account-create-oauth--mailbox-generate-flow" ||
          storyId ===
            "account-pool-pages-upstream-account-create-oauth--mailbox-generate-pending" ||
          storyId ===
            "account-pool-pages-upstream-account-create-batch-oauth--mailbox-generate-flow" ||
          storyId ===
            "account-pool-pages-upstream-account-create-batch-oauth--mailbox-generate-pending";
        if (requestedAddress && shouldDelayMailboxAttach) {
          await wait(900);
        }
        if (!requestedAddress && shouldDelayMailboxGenerate) {
          await wait(900);
        }
        if (requestedAddress) {
          if (!requestedAddress.includes("@")) {
            return jsonResponse(
              {
                supported: false,
                emailAddress: requestedAddress,
                reason: "invalid_format",
              },
              201,
            );
          }
          const isSupportedDomain = requestedAddress.endsWith("@mail-tw.707079.xyz");
          if (!isSupportedDomain) {
            return jsonResponse(
              {
                supported: false,
                emailAddress: requestedAddress,
                reason: "unsupported_domain",
              },
              201,
            );
          }
          const existingRemoteMailbox = storyHasExistingMailboxAddress(store, requestedAddress);
          const nextMailboxId = store.nextMailboxId++;
          const sessionId = `mailbox_${nextMailboxId}`;
          const expiresAt = storyFutureExpiresAt;
          store.mailboxStatuses[sessionId] = {
            sessionId,
            emailAddress: requestedAddress,
            expiresAt,
            latestCode: null,
            invite: null,
            invited: false,
          };
          return jsonResponse(
            {
              supported: true,
              sessionId,
              emailAddress: requestedAddress,
              expiresAt,
              source: existingRemoteMailbox ? "attached" : "generated",
            },
            201,
          );
        }
        const nextMailboxId = store.nextMailboxId++;
        const sessionId = `mailbox_${nextMailboxId}`;
        const emailAddress = `storybook-oauth-${nextMailboxId}@mail-tw.707079.xyz`;
        const expiresAt = storyFutureExpiresAt;
        store.mailboxStatuses[sessionId] = {
          sessionId,
          emailAddress,
          expiresAt,
          latestCode: null,
          invite: null,
          invited: false,
        };
        return jsonResponse(
          {
            supported: true,
            sessionId,
            emailAddress,
            expiresAt,
            source: "generated",
          },
          201,
        );
      }

      if (
        path === "/api/pool/upstream-accounts/oauth/mailbox-sessions/status" &&
        method === "POST"
      ) {
        const body = parseBody<{ sessionIds?: string[] }>(init?.body, {});
        const sessionIds = Array.isArray(body.sessionIds) ? body.sessionIds : [];
        const items = sessionIds
          .map((sessionId) => store.mailboxStatuses[sessionId])
          .filter((item): item is OauthMailboxStatus => item != null);
        return jsonResponse({ items });
      }

      const mailboxSessionMatch = path.match(
        /^\/api\/pool\/upstream-accounts\/oauth\/mailbox-sessions\/([^/]+)$/,
      );
      if (mailboxSessionMatch && method === "DELETE") {
        const sessionId = decodeURIComponent(mailboxSessionMatch[1]);
        delete store.mailboxStatuses[sessionId];
        return noContent();
      }

      const loginSessionMatch = path.match(
        /^\/api\/pool\/upstream-accounts\/oauth\/login-sessions\/([^/]+)$/,
      );
      if (loginSessionMatch && method === "PATCH") {
        const loginId = decodeURIComponent(loginSessionMatch[1]);
        const session = store.sessions[loginId];
        if (!session) return jsonResponse({ message: "missing mock session" }, 404);
        const body = parseBody<UpdateOauthLoginSessionPayload>(init?.body, {});
        session.displayName = body.displayName?.trim() || undefined;
        session.email = body.email?.trim() || undefined;
        session.groupName = body.groupName?.trim() || undefined;
        session.note = body.note?.trim() || undefined;
        session.groupNote = body.groupNote?.trim() || undefined;
        session.tagIds = Array.isArray(body.tagIds) ? body.tagIds : [];
        session.isMother = body.isMother === true;
        session.mailboxSessionId = body.mailboxSessionId?.trim() || undefined;
        session.mailboxAddress = body.mailboxAddress?.trim() || undefined;
        return jsonResponse(clone(session));
      }
      if (loginSessionMatch && method === "GET") {
        const loginId = decodeURIComponent(loginSessionMatch[1]);
        const session = store.sessions[loginId];
        if (!session) return jsonResponse({ message: "missing mock session" }, 404);
        return jsonResponse(clone(session));
      }

      const completeLoginSessionMatch = path.match(
        /^\/api\/pool\/upstream-accounts\/oauth\/login-sessions\/([^/]+)\/complete$/,
      );
      const confirmIdentityMatch = path.match(
        /^\/api\/pool\/upstream-accounts\/oauth\/login-sessions\/([^/]+)\/confirm-identity-overwrite$/,
      );
      if (confirmIdentityMatch && method === "POST") {
        const loginId = decodeURIComponent(confirmIdentityMatch[1]);
        const session = store.sessions[loginId];
        if (!session) return jsonResponse({ message: "missing mock session" }, 404);
        if (session.status !== "needs_identity_confirmation") {
          return jsonResponse({ message: "session is not waiting for identity confirmation" }, 400);
        }
        const nextId = session.accountId ?? store.nextId++;
        const existing = store.details[nextId];
        const incoming = session.identityConfirmation?.incoming;
        const detail = createOauthAccount(nextId, {
          displayName: existing?.displayName ?? session.displayName ?? "Relogin target",
          email: existing?.email ?? session.email ?? "kept@example.com",
          verifiedEmail: incoming?.verifiedEmail ?? incoming?.email ?? "incoming@example.com",
          groupName: existing?.groupName ?? session.groupName ?? "default",
          isMother: existing?.isMother ?? session.isMother ?? false,
          note: existing?.note ?? session.note ?? null,
          planType: incoming?.planType ?? existing?.planType ?? "team",
        });
        store.details[nextId] = detail;
        store.accounts = [
          toSummary(detail),
          ...store.accounts.filter((item) => item.id !== nextId),
        ];
        session.accountId = nextId;
        session.status = "completed";
        session.authUrl = null;
        session.redirectUri = null;
        session.error = null;
        session.identityConfirmation = null;
        return jsonResponse(clone(detail));
      }
      if (completeLoginSessionMatch && method === "POST") {
        const loginId = decodeURIComponent(completeLoginSessionMatch[1]);
        const session = store.sessions[loginId];
        if (!session) return jsonResponse({ message: "missing mock session" }, 404);
        const body = parseBody<CompleteOauthLoginSessionPayload>(init?.body, {
          callbackUrl: "",
        });
        const callbackUrl = body.callbackUrl.trim();
        if (!callbackUrl || !session.state || !callbackUrl.includes(session.state)) {
          session.status = "failed";
          session.error = "Mock callback URL does not contain the expected state token.";
          return jsonResponse({ message: session.error }, 400);
        }
        const nextId = session.accountId ?? store.nextId++;
        const existing = store.details[nextId];
        const emailChoiceStory =
          storyId === "account-pool-pages-upstream-account-create-oauth--completed-email-choice";
        const chosenEmail = session.email?.trim() || existing?.email || "new-login@example.com";
        const verifiedEmail = emailChoiceStory
          ? "verified@storybook.example.com"
          : (existing?.verifiedEmail ?? chosenEmail);
        const detail = createOauthAccount(nextId, {
          displayName: session.displayName || existing?.displayName || "Codex Pro - New login",
          email: chosenEmail,
          verifiedEmail,
          groupName: session.groupName ?? existing?.groupName ?? "default",
          isMother: session.isMother ?? existing?.isMother ?? false,
          note: session.note ?? existing?.note ?? "Freshly connected from Storybook OAuth mock.",
        });
        const normalizedGroupName = normalizeGroupName(detail.groupName);
        if (normalizedGroupName && session.groupNote?.trim()) {
          store.groupNotes[normalizedGroupName] = session.groupNote.trim();
        }
        store.details[nextId] = detail;
        const summary = toSummary(detail);
        store.accounts = [summary, ...store.accounts.filter((item) => item.id !== nextId)];
        session.accountId = nextId;
        session.status = "completed";
        session.authUrl = null;
        session.redirectUri = null;
        session.error = null;
        return jsonResponse(clone(detail));
      }

      if (path === "/api/pool/upstream-accounts/api-keys" && method === "POST") {
        const body = parseBody<CreateApiKeyAccountPayload>(init?.body, {
          displayName: "New API key",
          apiKey: "sk-storybook-key",
        });
        const nextId = store.nextId++;
        const detail = createApiKeyAccount(nextId, {
          displayName: body.displayName,
          email: body.email ?? null,
          groupName: body.groupName ?? "default",
          isMother: body.isMother === true,
          note: body.note ?? null,
          upstreamBaseUrl: body.upstreamBaseUrl ?? null,
          maskedApiKey: maskApiKey(body.apiKey),
          localLimits: {
            primaryLimit: body.localPrimaryLimit ?? 120,
            secondaryLimit: body.localSecondaryLimit ?? 500,
            limitUnit: body.localLimitUnit ?? "requests",
          },
        });
        const synced = syncLocalWindows(detail);
        const normalizedGroupName = normalizeGroupName(synced.groupName);
        if (normalizedGroupName && body.groupNote?.trim()) {
          store.groupNotes[normalizedGroupName] = body.groupNote.trim();
        }
        store.details[nextId] = synced;
        store.accounts = [toSummary(synced), ...store.accounts];
        return jsonResponse(clone(synced), 201);
      }

      const reloginMatch = path.match(/^\/api\/pool\/upstream-accounts\/(\d+)\/oauth\/relogin$/);
      if (reloginMatch && method === "POST") {
        const accountId = Number(reloginMatch[1]);
        const state = `state_relogin_${accountId}`;
        const session: StoryStore["sessions"][string] = {
          loginId: `relogin_${accountId}_${Date.now()}`,
          status: "pending",
          authUrl: `https://auth.openai.com/authorize?mock=1&accountId=${accountId}&state=${state}`,
          redirectUri: `http://localhost:432${String(accountId).slice(-1)}/oauth/callback`,
          expiresAt: storyFutureLoginExpiresAt,
          accountId,
          error: null,
          state,
        };
        store.sessions[session.loginId] = session;
        return jsonResponse(clone(session), 201);
      }

      const syncMatch = path.match(/^\/api\/pool\/upstream-accounts\/(\d+)\/sync$/);
      if (syncMatch && method === "POST") {
        const accountId = Number(syncMatch[1]);
        const detail = store.details[accountId];
        if (!detail) return jsonResponse({ message: "missing mock account" }, 404);
        const updated = syncLocalWindows({
          ...detail,
          status: "active",
          lastSyncedAt: now,
          lastSuccessfulSyncAt: now,
          lastError: null,
          lastErrorAt: null,
        });
        store.details[accountId] = updated;
        store.accounts = store.accounts.map((item) =>
          item.id === accountId ? toSummary(updated) : item,
        );
        return jsonResponse(clone(updated));
      }

      const detailMatch = path.match(/^\/api\/pool\/upstream-accounts\/(\d+)$/);
      if (detailMatch && method === "GET") {
        const accountId = Number(detailMatch[1]);
        const detail = store.details[accountId];
        if (!detail) return jsonResponse({ message: "missing mock account" }, 404);
        return jsonResponse(clone(detail));
      }

      const attemptMatch = path.match(
        /^\/api\/pool\/upstream-accounts\/(\d+)\/call-attempts(?:\/locate)?$/,
      );
      if (attemptMatch && method === "GET") {
        const accountId = Number(attemptMatch[1]);
        const attemptId = parsedUrl.searchParams.get("attemptId")?.trim() || "4V7MYPJG";
        const attempts: ApiPoolUpstreamRequestAttempt[] = [
          {
            attemptId: "4V7MYPJG",
            invokeId: "storybook-pool-retry-001",
            occurredAt: "2026-07-11T12:00:00.000Z",
            endpoint: "/v1/responses",
            stickyKey: "storybook-sticky",
            upstreamAccountId: accountId,
            upstreamAccountName: "Codex Pro - Tokyo",
            model: "gpt-5.4",
            requestModel: "gpt-5.4",
            proxyBindingKeySnapshot: "jp-edge-01",
            requesterIp: "203.0.113.24",
            connectLatencyMs: 186,
            firstByteLatencyMs: 742,
            streamLatencyMs: 1284,
            downstreamRequestContentEncoding: "gzip",
            upstreamRequestCompressionAlgorithm: "zstd",
            upstreamRequestCompressionMode: "recompressed",
            logicalBodyBytes: 1000,
            transmittedBodyBytes: 580,
            savedBytes: 420,
            ratioPct: -42,
            approxUploadBytes: 644,
            approxDownloadBytes: 812,
            upstreamRequestId: "upstream-story-500",
            upstreamRouteKey: "route-tokyo-primary",
            attemptIndex: 1,
            distinctAccountIndex: 0,
            sameAccountRetryIndex: 0,
            status: "http_failure",
            phase: "failed",
            httpStatus: 500,
            downstreamHttpStatus: 502,
            failureKind: "upstream_response_failed",
            errorMessage: "pool upstream responded with 500",
            createdAt: "2026-07-11T12:00:00.000Z",
          },
          {
            attemptId: "QADKN5Z9",
            invokeId: "storybook-pool-retry-001",
            occurredAt: "2026-07-11T12:00:01.000Z",
            endpoint: "/v1/responses",
            stickyKey: "storybook-sticky",
            upstreamAccountId: accountId,
            upstreamAccountName: "Codex Pro - Tokyo",
            model: "gpt-5.4",
            requestModel: "gpt-5.4",
            responseModel: "gpt-5.4-2026-07-01",
            proxyBindingKeySnapshot: "jp-edge-01",
            requesterIp: "203.0.113.24",
            connectLatencyMs: 94,
            firstByteLatencyMs: 328,
            streamLatencyMs: 1890,
            downstreamRequestContentEncoding: "identity",
            upstreamRequestCompressionAlgorithm: "identity",
            upstreamRequestCompressionMode: "identity",
            logicalBodyBytes: 612,
            transmittedBodyBytes: 612,
            savedBytes: 0,
            ratioPct: 0,
            approxUploadBytes: 676,
            approxDownloadBytes: 2048,
            upstreamRequestId: "upstream-story-200",
            upstreamRouteKey: "route-tokyo-primary",
            attemptIndex: 2,
            distinctAccountIndex: 0,
            sameAccountRetryIndex: 1,
            status: "success",
            phase: "completed",
            httpStatus: 200,
            createdAt: "2026-07-11T12:00:01.000Z",
          },
        ];
        if (storyId?.endsWith("--detail-drawer-records-pending-mobile")) {
          attempts.unshift({
            attemptId: "YG7P25XG",
            invokeId: "storybook-pool-pending-001",
            occurredAt: "2026-07-11T12:00:02.000Z",
            endpoint: "/v1/responses",
            upstreamAccountId: accountId,
            upstreamAccountName: "Codex Pro - Tokyo",
            model: "gpt-5.4",
            requestModel: "gpt-5.4",
            proxyBindingKeySnapshot: "jp-edge-01",
            attemptIndex: 1,
            distinctAccountIndex: 0,
            sameAccountRetryIndex: 0,
            status: "pending",
            phase: "waiting_first_byte",
            connectLatencyMs: 83,
            createdAt: "2026-07-11T12:00:02.000Z",
          });
        }
        if (
          path.endsWith("/locate") &&
          !attempts.some((attempt) => attempt.attemptId === attemptId)
        ) {
          return jsonResponse({ message: "upstream account attempt was not found" }, 404);
        }
        return jsonResponse({ items: attempts, total: attempts.length, page: 1, pageSize: 50 });
      }

      if (path === "/api/invocations/locate" && method === "GET") {
        const requestedAccountId = Number(parsedUrl.searchParams.get("upstreamAccountId") || 0);
        const requestId = parsedUrl.searchParams.get("requestId")?.trim() || "";
        if (storyId?.endsWith("--detail-drawer-invocation-locate-not-found")) {
          return jsonResponse(
            {
              code: "invocation_not_found",
              message: "invocation record not found",
              requestId,
            },
            404,
          );
        }
        const records = buildStickyInvocationRecords(
          requestedAccountId > 0 ? requestedAccountId : 101,
        );
        const target = {
          ...(records[0] ?? {}),
          id: 910_001,
          invokeId: requestId,
          upstreamAccountId: requestedAccountId || 101,
        };
        const targetIndex = Math.min(3, records.length);
        return jsonResponse({
          anchorId: "storybook-anchor-001",
          snapshotId: 17,
          total: 121,
          page: 2,
          pageSize: 50,
          records: [...records.slice(0, targetIndex), target, ...records.slice(targetIndex + 1, 8)],
          targetIndex,
          targetAbsoluteIndex: 50 + targetIndex,
        });
      }

      if (path === "/api/invocations" && method === "GET") {
        const requestedAccountId = Number(parsedUrl.searchParams.get("upstreamAccountId") || 0);
        const stickyKey = parsedUrl.searchParams.get("stickyKey")?.trim() || "";
        const pageSize = Math.max(1, Number(parsedUrl.searchParams.get("pageSize") || 20));
        const page = Math.max(1, Number(parsedUrl.searchParams.get("page") || 1));
        const allRecords = storyAccountActivityIsEmpty(storyId)
          ? []
          : buildStickyInvocationRecords(requestedAccountId > 0 ? requestedAccountId : 101);
        const filteredRecords = allRecords.filter(
          (record) =>
            (requestedAccountId > 0 ? record.upstreamAccountId === requestedAccountId : true) &&
            (stickyKey ? record.promptCacheKey === stickyKey : true),
        );
        const start = (page - 1) * pageSize;
        return jsonResponse({
          snapshotId: 1,
          total: filteredRecords.length,
          page,
          pageSize,
          records: filteredRecords.slice(start, start + pageSize),
        });
      }

      const stickyMatch = path.match(/^\/api\/pool\/upstream-accounts\/(\d+)\/sticky-keys$/);
      if (stickyMatch && method === "GET") {
        const accountId = Number(stickyMatch[1]);
        return jsonResponse(buildStickyConversations(accountId, parsedUrl));
      }

      if (detailMatch && method === "PATCH") {
        if (storyId?.endsWith("--completed-save-failure-feedback")) {
          return Promise.resolve(
            new Response("Storybook forced save failure.", {
              status: 500,
              headers: { "Content-Type": "text/plain" },
            }),
          );
        }
        const accountId = Number(detailMatch[1]);
        const detail = store.details[accountId];
        if (!detail) return jsonResponse({ message: "missing mock account" }, 404);
        const body = parseBody<UpdateUpstreamAccountPayload>(init?.body, {});
        const nextEmail = Object.hasOwn(body, "email") ? (body.email ?? null) : detail.email;
        const nextEffectiveRoutingRule = body.routingRule
          ? applyRoutingRulePatchToEffectiveRule(detail.effectiveRoutingRule, body.routingRule)
          : detail.effectiveRoutingRule;
        const nextBoundProxyKeys = Object.hasOwn(body, "boundProxyKeys")
          ? Array.isArray(body.boundProxyKeys)
            ? Array.from(new Set(body.boundProxyKeys.map((value) => value.trim()).filter(Boolean)))
            : []
          : (detail.boundProxyKeys ?? []);
        const updated = syncLocalWindows({
          ...detail,
          displayName:
            body.displayName ??
            resolveDisplayNameAfterEmailChange(detail.displayName, detail.email, nextEmail),
          email: nextEmail,
          groupName: body.groupName ?? detail.groupName,
          isMother: body.isMother ?? detail.isMother,
          note: body.note ?? detail.note,
          upstreamBaseUrl:
            detail.kind === "api_key_codex" && Object.hasOwn(body, "upstreamBaseUrl")
              ? (body.upstreamBaseUrl ?? null)
              : detail.upstreamBaseUrl,
          enabled: body.enabled ?? detail.enabled,
          status:
            body.enabled === false
              ? "disabled"
              : detail.status === "disabled"
                ? "active"
                : detail.status,
          maskedApiKey: body.apiKey ? maskApiKey(body.apiKey) : detail.maskedApiKey,
          boundProxyKeys: nextBoundProxyKeys,
          localLimits:
            detail.kind === "api_key_codex"
              ? {
                  primaryLimit: body.localPrimaryLimit ?? detail.localLimits?.primaryLimit ?? 120,
                  secondaryLimit:
                    body.localSecondaryLimit ?? detail.localLimits?.secondaryLimit ?? 500,
                  limitUnit: body.localLimitUnit ?? detail.localLimits?.limitUnit ?? "requests",
                }
              : detail.localLimits,
          effectiveRoutingRule: nextEffectiveRoutingRule,
        });
        store.details[accountId] = updated;
        store.accounts = store.accounts.map((item) =>
          item.id === accountId ? toSummary(updated) : item,
        );
        return jsonResponse(clone(updated));
      }

      const groupMatch = path.match(/^\/api\/pool\/upstream-account-groups\/(.+)$/);
      if (groupMatch && method === "PUT") {
        const groupName = decodeURIComponent(groupMatch[1]);
        const body = parseBody<UpdateUpstreamAccountGroupPayload>(init?.body, {});
        const normalized = normalizeGroupName(groupName);
        if (!normalized) return jsonResponse({ message: "missing mock group" }, 404);
        const note = body.note?.trim() ?? "";
        const boundProxyKeys = Array.isArray(body.boundProxyKeys)
          ? Array.from(new Set(body.boundProxyKeys.map((value) => value.trim()).filter(Boolean)))
          : [];
        if (note) store.groupNotes[normalized] = note;
        else delete store.groupNotes[normalized];
        if (boundProxyKeys.length > 0) {
          store.groupBoundProxyKeys[normalized] = boundProxyKeys;
        } else {
          delete store.groupBoundProxyKeys[normalized];
        }
        return jsonResponse({
          groupName: normalized,
          note: note || null,
          boundProxyKeys,
          routingRule: body.routingRule,
        });
      }

      if (detailMatch && method === "DELETE") {
        const accountId = Number(detailMatch[1]);
        if (storyId?.endsWith("--delete-failure")) {
          return Promise.resolve(
            new Response("error returned from database: (code: 5) database is locked", {
              status: 500,
              headers: { "Content-Type": "text/plain" },
            }),
          );
        }
        delete store.details[accountId];
        store.accounts = store.accounts.filter((item) => item.id !== accountId);
        return noContent();
      }

      return (originalFetchRef.current as typeof window.fetch)(input, init);
    };

    window.fetch = mockedFetch;
    window.EventSource = class extends MockStoryBulkSyncEventSource {
      constructor(url: string | URL) {
        super(storeRef, url);
      }
    } as unknown as typeof window.EventSource;
  }

  useEffect(() => {
    return () => {
      if (typeof window !== "undefined") {
        delete window.__storybookUpstreamAccountsController__;
      }
      if (typeof window !== "undefined" && originalFetchRef.current) {
        window.fetch = originalFetchRef.current;
        originalFetchRef.current = null;
      }
      if (typeof window !== "undefined" && originalEventSourceRef.current) {
        window.EventSource = originalEventSourceRef.current;
        originalEventSourceRef.current = null;
      }
    };
  }, []);

  return <>{children}</>;
}

export function AccountPoolStoryRouter({ initialEntry }: { initialEntry: StoryInitialEntry }) {
  const { themeMode } = useTheme();
  const isDark = themeMode === "dark";
  return (
    <div
      className="min-h-screen bg-base-200 px-6 py-6 text-base-content"
      style={{
        backgroundImage: isDark
          ? "radial-gradient(circle at 10% -10%, rgba(56,189,248,0.18), transparent 36%), radial-gradient(circle at 88% 0%, rgba(45,212,191,0.16), transparent 34%), linear-gradient(180deg, #081428 0%, #10213a 62%)"
          : "radial-gradient(circle at 10% -10%, rgba(14,165,233,0.10), transparent 34%), radial-gradient(circle at 88% 0%, rgba(16,185,129,0.10), transparent 30%), linear-gradient(180deg, #f7fbff 0%, #e8f1fb 58%, #e1ecf8 100%)",
      }}
    >
      <MemoryRouter initialEntries={[initialEntry]}>
        <Routes>
          <Route path="/account-pool" element={<AccountPoolLayout />}>
            <Route path="upstream-accounts" element={<UpstreamAccountsPage />} />
            <Route path="upstream-accounts/new" element={<UpstreamAccountCreatePage />} />
            <Route path="maintenance-records" element={<MaintenanceRecordsPage />} />
            <Route path="groups" element={<GroupsPage />} />
          </Route>
        </Routes>
      </MemoryRouter>
    </div>
  );
}
